//! The Print Center's link to the scheduler. A thread owns the CUPS client, so a slow or stopped scheduler never stalls the
//! window: requests go in on one channel and replies come back on another, which the window drains on each tick.

use crate::model::{self, Action, Reply};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::controls::cups::{self, Client, Printer, Transport};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock};
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
    /// Add a queue for the device at this address (driverless). A non-empty `label` names the queue after the printer's
    /// advertised name and `info` and `location` are saved with it; an empty label names it from the address.
    Add { device: String, label: String, info: String, location: String },
    /// Print a document on `printer`: `name` is the job name, `mime` the document type.
    Print { printer: String, name: String, mime: String, data: Vec<u8> },
    /// Ask the device of `printer` for its options (paper, sides, colour) for the info pane.
    Options { printer: String },
}

/// The window's end of the serial dispatch queue and reply buffer.
pub struct Link {
    queue: DispatchQueue,
    client: Option<Arc<Mutex<Client<cups::HttpTransport>>>>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
}

impl Link {
    /// Starts a serial queue for requests to the local scheduler.
    pub fn start() -> Link {
        Link {
            queue: DispatchQueue::serial_qos("is.olibuijr.printers.scheduler", Qos::Utility),
            client: Some(Arc::new(Mutex::new(Client::local()))),
            replies: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// A link with no worker behind it: requests are dropped and no reply comes. For tests.
    #[cfg(test)]
    pub fn detached() -> Link {
        Link {
            queue: DispatchQueue::serial_qos("is.olibuijr.printers.detached", Qos::Utility),
            client: None,
            replies: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn send(&self, request: Request) {
        let (Some(client), replies) = (self.client.clone(), self.replies.clone()) else { return };
        self.queue.async_(move || {
            let reply = match client.lock() {
                Ok(mut client) => handle(&mut client, request),
                Err(_) => Reply::Failed("The print scheduler connection is unavailable".into()),
            };
            DispatchQueue::main().async_(move || {
                if let Ok(mut queue) = replies.lock() {
                    queue.push_back(reply);
                }
            });
        });
    }

    /// The replies that arrived since the last call.
    pub fn drain(&self) -> Vec<Reply> {
        self.replies.lock().map(|mut queue| queue.drain(..).collect()).unwrap_or_default()
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
        Request::Add { device, label, info, location } => match add_printer(client, &device, &label, &info, &location) {
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
        // a device that does not answer its attributes has no options to show, which is not an error to report
        Request::Options { printer } => {
            let options = client.get_printer_attributes(&printer).ok().map(|caps| model::Options {
                paper: caps.default_paper.ppd_name(),
                two_sided: caps.supports_duplex(),
                color: caps.supports_color(),
            });
            Reply::Options { printer, options }
        }
    }
}

/// Adds the queue and returns its name. Without a label the queue is named from the address (as Settings does); with one
/// it is named from the label and the description and location are saved, which a failed save leaves unsaved but does
/// not undo the queue.
fn add_printer<T: Transport>(client: &mut Client<T>, device: &str, label: &str, info: &str, location: &str) -> Result<String, cups::CupsError> {
    if label.trim().is_empty() {
        return cups::add_device(client, device);
    }
    let device = device.trim();
    if !cups::valid_device_uri(device) {
        return Err(cups::CupsError::BadDeviceUri(device.to_string()));
    }
    let taken: Vec<String> = client.printers()?.into_iter().map(|p| p.name).collect();
    let name = model::queue_name_for(label, &taken);
    client.add_driverless_printer(&name, device)?;
    if !info.is_empty() || !location.is_empty() {
        let _ = client.set_printer_details(&name, info, location);
    }
    Ok(name)
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
/// own utility-queue task (a name lookup can be slow), so the listing never waits for the network and shows the previous answer.
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
            DispatchQueue::global(Qos::Utility).async_(move || {
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

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::cups::CupsError;
    use bifrost_app::ipp::{self, group, Attribute, Group, Request as IppMessage, Value};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// Replies in order and keeps what was sent where the test can read it. Nothing touches the network.
    struct Canned {
        replies: VecDeque<Vec<u8>>,
        sent: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    impl Transport for Canned {
        fn send(&mut self, body: &[u8]) -> Result<Vec<u8>, CupsError> {
            self.sent.borrow_mut().push(body.to_vec());
            self.replies.pop_front().ok_or(CupsError::Io("no canned reply".into()))
        }
    }

    fn reply(status: u16, id: u32, groups: Vec<Group>) -> Vec<u8> {
        let mut r = IppMessage::new(status);
        r.request_id = id;
        r.groups = groups;
        r.encode()
    }

    fn printer_group(name: &str) -> Group {
        Group { tag: group::PRINTER, attributes: vec![Attribute { name: "printer-name".into(), values: vec![Value::name(name)] }] }
    }

    fn contains(body: &[u8], needle: &str) -> bool {
        body.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    /// A client whose scheduler already has a queue named Office, and the bodies it was sent.
    fn client_with_office(extra: Vec<Vec<u8>>) -> (Client<Canned>, Rc<RefCell<Vec<Vec<u8>>>>) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let mut replies = vec![reply(0, 1, vec![printer_group("Office")]), reply(0, 2, vec![printer_group("Office")]), reply(ipp::STATUS_NOT_FOUND, 3, vec![])];
        replies.extend(extra);
        (Client::new(Canned { replies: replies.into(), sent: sent.clone() }), sent)
    }

    #[test]
    fn a_found_printer_is_added_under_its_own_name_with_its_description_and_location() {
        let (mut client, sent) = client_with_office(vec![reply(0, 4, vec![]), reply(0, 5, vec![])]);
        let name = add_printer(&mut client, " ipps://10.0.0.7:631/ipp/print ", "Office", "Acme Laser 5", "Floor 2").unwrap();
        assert_eq!(name, "Office-2", "a name in use is never replaced");
        let sent = sent.borrow();
        assert_eq!(sent.len(), 5);
        assert!(contains(&sent[3], "Office-2") && contains(&sent[3], "ipps://10.0.0.7:631/ipp/print"));
        assert!(contains(&sent[4], "Acme Laser 5") && contains(&sent[4], "Floor 2"));
    }

    #[test]
    fn an_added_printer_without_details_saves_none_and_a_failed_save_keeps_the_queue() {
        let (mut client, sent) = client_with_office(vec![reply(0, 4, vec![])]);
        assert_eq!(add_printer(&mut client, "ipp://10.0.0.7:631/ipp/print", "Lab", "", "").unwrap(), "Lab");
        assert_eq!(sent.borrow().len(), 4, "no details, so no save request");
        let (mut client, _) = client_with_office(vec![reply(0, 4, vec![]), reply(0x0500, 5, vec![])]);
        assert_eq!(add_printer(&mut client, "ipp://10.0.0.7:631/ipp/print", "Lab", "Acme", ""), Ok("Lab".to_string()));
    }

    #[test]
    fn without_a_label_the_queue_is_named_from_the_address() {
        let (mut client, _) = client_with_office(vec![reply(0, 4, vec![])]);
        assert_eq!(add_printer(&mut client, "ipp://10.0.0.7:631/ipp/print", "  ", "", "").unwrap(), "10-0-0-7");
    }

    #[test]
    fn an_address_the_scheduler_does_not_take_is_refused_before_any_request() {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let mut client = Client::new(Canned { replies: VecDeque::new(), sent: sent.clone() });
        assert_eq!(add_printer(&mut client, "ftp://x", "Lab", "", ""), Err(cups::CupsError::BadDeviceUri("ftp://x".into())));
        assert!(sent.borrow().is_empty());
    }
}
