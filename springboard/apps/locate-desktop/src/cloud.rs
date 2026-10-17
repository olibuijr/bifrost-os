//! The cloud worker: one background thread that talks to the BifrOSt Cloud Find My routes for the window. The window
//! sends `Job`s (publish the key, approve or deny a pending device, store a report, clear the stored position) and
//! reads `Event`s back. The thread registers this device at start, long-polls the device's commands, and about once a
//! minute reads the device list, pending devices, shared positions and the account's event feed.
//!
//! Events reach the window through a channel; the thread writes one byte to a pipe the window polls (`Link::fd`), so
//! the window's `tick` runs when there is something to read. Every request is blocking and runs on this thread only.

use crate::pairing::PollAnswer;
use crate::sealing::Sealed;
use bifrost_app::cloud_account::{session_send, CloudError};
use bifrost_app::cloud_findmy::{self as fm, DeviceList, PendingCommand, PendingDevice, SharedLocation};
use bifrost_app::url::URLRequest;
use bifrost_core::plist::base64_encode;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

/// The Find My routes of the BifrOSt Cloud API.
pub const BASE_URL: &str = "https://bifrost.olibuijr.com/api/cloud/v1";
/// How long one command poll waits on the server before it returns.
const POLL_WAIT_SECS: u32 = 25;
/// How often the device list, pending devices, shared positions and the event feed are read.
const SYNC_EVERY: Duration = Duration::from_secs(60);
/// The pause after a command poll that failed, so an unreachable server is not hammered.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// What the window asks the worker to send.
pub enum Job {
    /// Publish the account public key.
    PublishKey([u8; 32]),
    /// Ask to join the account with this device's public key. The worker then polls for the sealed key.
    Enroll { device_public: [u8; 32], name: String },
    /// Hand the sealed account key to a pending device (`pairing::seal_for`, packed with `sealing::pack`).
    Approve { id: String, sealed_key: String },
    /// Deny a pending device.
    Deny { id: String },
    /// Store this device's sealed position, and a copy sealed to each live contact share.
    Report { own: Sealed, copies: Vec<(String, Sealed)> },
    /// Delete this device's stored position from the cloud (sharing is off).
    Clear,
}

/// What the worker tells the window.
pub enum Event {
    /// The device is registered on the account.
    Registered,
    /// The owner asked for a locate: report now.
    Locate,
    /// The owner asked for a sound on this device.
    PlaySound,
    /// The account's device list, with the sealed positions the cloud holds.
    Devices(DeviceList),
    /// The devices waiting for approval on the account.
    Pending(Vec<PendingDevice>),
    /// The positions other people share with the account.
    Shared(Vec<SharedLocation>),
    /// A join request was created under this id; the worker now polls for its key.
    Enrolled(String),
    /// The answer to the sealed-key poll of a joining device.
    SealedKey(PollAnswer),
    /// An event from the account's feed, to show as a notification.
    Notify { title: String, body: String },
    /// A request that failed, for the log.
    Failed(String),
}

/// The window's side of the worker: jobs in, events out, and the pipe that wakes the window.
pub struct Link {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    wake: UnixStream,
}

impl Link {
    /// Starts the worker for the session `token`, registering the device as `name`.
    pub fn start(token: String, name: String) -> std::io::Result<Link> {
        let (jobs, job_rx) = channel();
        let (event_tx, events) = channel();
        let (wake, pipe_tx) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        pipe_tx.set_nonblocking(true)?;
        thread::Builder::new().name("locate-cloud".into()).spawn(move || run(&token, &name, job_rx, event_tx, pipe_tx))?;
        Ok(Link { jobs, events, wake })
    }

    /// The descriptor the window waits on; it is readable when an event is waiting.
    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    pub fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// Empties the wake pipe and returns the events that arrived.
    pub fn drain(&mut self) -> Vec<Event> {
        let mut sink = [0u8; 64];
        while matches!(self.wake.read(&mut sink), Ok(n) if n > 0) {}
        self.events.try_iter().collect()
    }
}

/// The notification an account feed event shows, if the kind has one.
pub fn notice(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "command_done" => Some(("Locate", "A device finished your request.")),
        "share_received" => Some(("Location shared with you", "Someone shared their location with you.")),
        "share_ended" => Some(("Location sharing ended", "A location share with you has ended.")),
        _ => None,
    }
}

struct Worker {
    token: String,
    events: Sender<Event>,
    pipe: UnixStream,
    /// The pending id this device polls for its sealed key.
    watch: Option<String>,
    /// The last event sequence number read.
    after: i64,
}

fn run(token: &str, name: &str, jobs: Receiver<Job>, events: Sender<Event>, pipe: UnixStream) {
    let mut w = Worker { token: token.to_string(), events, pipe, watch: None, after: 0 };
    let register = fm::report_request(BASE_URL, token, Some(name), None, None);
    if w.run_once("register", register, fm::parse_report).is_some() {
        w.emit(Event::Registered);
    }
    let mut last_sync: Option<Instant> = None;
    loop {
        loop {
            match jobs.try_recv() {
                Ok(job) => w.job(job),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // The command poll is the long wait: the server holds it up to POLL_WAIT_SECS.
        if !w.poll_commands() {
            thread::sleep(RETRY_AFTER);
        }
        w.watch_key();
        if last_sync.map_or(true, |t| t.elapsed() >= SYNC_EVERY) {
            w.sync();
            last_sync = Some(Instant::now());
        }
    }
}

impl Worker {
    fn emit(&mut self, event: Event) {
        if self.events.send(event).is_ok() {
            let _ = (&self.pipe).write(&[1]);
        }
    }

    /// Sends one request and returns its status and body.
    fn send(&self, req: Result<URLRequest, CloudError>) -> Result<(u16, String), String> {
        let req = req.map_err(|e| format!("{e:?}"))?;
        let reply = session_send(&req)?;
        Ok((reply.status, reply.body))
    }

    /// Sends one request and parses the answer. A failure is reported as an event and gives `None`.
    fn run_once<T>(&mut self, what: &str, req: Result<URLRequest, CloudError>, parse: impl FnOnce(u16, &str) -> Result<T, CloudError>) -> Option<T> {
        let (status, body) = match self.send(req) {
            Ok(answer) => answer,
            Err(e) => {
                self.emit(Event::Failed(format!("{what}: {e}")));
                return None;
            }
        };
        match parse(status, &body) {
            Ok(value) => Some(value),
            Err(e) => {
                self.emit(Event::Failed(format!("{what}: {e:?}")));
                None
            }
        }
    }

    fn job(&mut self, job: Job) {
        match job {
            Job::PublishKey(public) => {
                let req = fm::put_account_key_request(BASE_URL, &self.token, &base64_encode(&public));
                let _ = self.run_once("publish key", req, fm::parse_approve_pending);
            }
            Job::Enroll { device_public, name } => {
                let req = fm::create_pending_request(BASE_URL, &self.token, &base64_encode(&device_public), &name);
                if let Some(id) = self.run_once("enroll", req, fm::parse_pending_id) {
                    self.watch = Some(id.clone());
                    self.emit(Event::Enrolled(id));
                }
            }
            Job::Approve { id, sealed_key } => {
                let req = fm::approve_pending_request(BASE_URL, &self.token, &id, &sealed_key);
                let _ = self.run_once("approve", req, fm::parse_approve_pending);
            }
            Job::Deny { id } => {
                let req = fm::delete_pending_request(BASE_URL, &self.token, &id);
                let _ = self.run_once("deny", req, fm::parse_delete);
            }
            Job::Report { own, copies } => {
                let req = fm::put_self_location_request(BASE_URL, &self.token, &own.sealed, &own.enc);
                let _ = self.run_once("report", req, fm::parse_approve_pending);
                for (share_id, blob) in copies {
                    let req = fm::put_share_location_request(BASE_URL, &self.token, &share_id, &blob.sealed, &blob.enc);
                    let _ = self.run_once("share copy", req, fm::parse_approve_pending);
                }
            }
            Job::Clear => {
                let req = fm::delete_self_location_request(BASE_URL, &self.token);
                let _ = self.run_once("clear", req, fm::parse_delete);
            }
        }
    }

    /// One command poll. Each command is handed to the window and acknowledged here: `done` for a locate or a sound,
    /// `failed` for a kind this device does not know. Returns false when the poll itself failed.
    fn poll_commands(&mut self) -> bool {
        let req = fm::commands_request(BASE_URL, &self.token, POLL_WAIT_SECS);
        let Some(commands) = self.run_once("commands", req, fm::parse_commands) else { return false };
        for c in commands {
            self.command(c);
        }
        true
    }

    fn command(&mut self, c: PendingCommand) {
        let ok = match c.kind.as_str() {
            "locate" => {
                self.emit(Event::Locate);
                true
            }
            "play_sound" => {
                self.emit(Event::PlaySound);
                true
            }
            _ => false,
        };
        let req = fm::ack_request(BASE_URL, &self.token, &c.id, ok);
        let _ = self.run_once("ack", req, fm::parse_approve_pending);
    }

    /// The sealed-key poll of a joining device. A sealed answer or a gone request ends the watch; a wait keeps it.
    fn watch_key(&mut self) {
        let Some(id) = self.watch.clone() else { return };
        let req = fm::sealed_key_request(BASE_URL, &self.token, &id);
        let answer = match self.send(req) {
            Ok((status, body)) => match fm::parse_sealed_key(status, &body) {
                Ok(Some(blob)) => PollAnswer::Sealed(blob),
                Ok(None) => PollAnswer::StillWaiting,
                Err(CloudError::Rejected { status: 404, .. }) => PollAnswer::Gone,
                Err(e) => {
                    self.emit(Event::Failed(format!("sealed key: {e:?}")));
                    PollAnswer::StillWaiting
                }
            },
            Err(e) => {
                self.emit(Event::Failed(format!("sealed key: {e}")));
                PollAnswer::StillWaiting
            }
        };
        if answer != PollAnswer::StillWaiting {
            self.watch = None;
            self.emit(Event::SealedKey(answer));
        }
    }

    /// The periodic reads: devices, pending devices, shared positions, and the feed's new events as notices.
    fn sync(&mut self) {
        let req = fm::devices_request(BASE_URL, &self.token);
        if let Some(list) = self.run_once("devices", req, fm::parse_devices) {
            self.emit(Event::Devices(list));
        }
        let req = fm::pending_list_request(BASE_URL, &self.token);
        if let Some(list) = self.run_once("pending", req, fm::parse_pending_list) {
            self.emit(Event::Pending(list));
        }
        let req = fm::shared_with_me_request(BASE_URL, &self.token);
        if let Some(list) = self.run_once("shared", req, fm::parse_shared_with_me) {
            self.emit(Event::Shared(list));
        }
        let req = fm::events_request(BASE_URL, &self.token, self.after, 0);
        if let Some(page) = self.run_once("events", req, fm::parse_events) {
            for event in page.events {
                if let Some((title, body)) = notice(&event.kind) {
                    self.emit(Event::Notify { title: title.into(), body: body.into() });
                }
            }
            self.after = self.after.max(page.next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feed_events_that_notify_have_a_notice() {
        assert_eq!(notice("command_done").map(|n| n.0), Some("Locate"));
        assert!(notice("share_received").is_some());
        assert!(notice("share_ended").is_some());
        assert_eq!(notice("share"), None, "the owner's own share does not notify");
        assert_eq!(notice("command"), None);
    }
}
