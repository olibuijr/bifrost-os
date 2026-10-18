//! Printer management over the CUPS scheduler's IPP interface (`ipp://localhost:631`).
//!
//! [`Client`] builds the operations with [`crate::ipp`] and reads back [`Printer`] and [`Job`] models: list printers
//! and the default, list queue jobs, pause and resume a queue, cancel a job, set the default, add a driverless
//! (IPP Everywhere) printer, delete a printer, and submit a document with Print-Job. The wire is behind the
//! [`Transport`] trait; [`HttpTransport`] speaks HTTP/1.1 to the scheduler, and the tests use a canned transport, so
//! no test touches the network.

use crate::ipp::{self, group, op, Group, IppError, Request, Response, Value};
use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const DEFAULT_HOST: &str = "localhost";
pub const DEFAULT_PORT: u16 = 631;
const TIMEOUT: Duration = Duration::from_secs(10);

/// The attributes a printer listing asks for.
const CAPABILITY_ATTRIBUTES: &[&str] = &[
    "media-supported",
    "media-default",
    "sides-supported",
    "print-color-mode-supported",
    "printer-state",
    "printer-state-reasons",
    "printer-state-message",
];

const PRINTER_ATTRIBUTES: &[&str] = &[
    "printer-name",
    "printer-uri-supported",
    "printer-state",
    "printer-state-message",
    "printer-is-accepting-jobs",
    "printer-make-and-model",
    "printer-location",
    "printer-info",
    "device-uri",
    "queued-job-count",
    "printer-state-reasons",
    "printer-is-shared",
    "marker-names",
    "marker-levels",
    "marker-colors",
    "member-names",
];

const JOB_ATTRIBUTES: &[&str] = &[
    "job-id",
    "job-uri",
    "job-state",
    "job-name",
    "job-originating-user-name",
    "job-k-octets",
    "job-state-reasons",
    "time-at-creation",
    "job-impressions",
    "job-impressions-completed",
    "job-media-progress",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CupsError {
    /// The socket failed or the peer closed early.
    Io(String),
    /// The HTTP framing was not usable.
    Http(String),
    /// The IPP body did not decode.
    Protocol(IppError),
    /// The scheduler answered with an IPP error status.
    Status(u16),
    /// A reply lacked an attribute the operation needs.
    Missing(&'static str),
    /// A printer name outside the scheduler's allowed set (letters, digits, `-`, `_`, `.`).
    BadName(String),
    /// A device URI with a scheme this client does not add.
    BadDeviceUri(String),
}

impl fmt::Display for CupsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CupsError::Io(e) => write!(f, "printing service unreachable: {e}"),
            CupsError::Http(e) => write!(f, "printing service HTTP error: {e}"),
            CupsError::Protocol(e) => write!(f, "{e}"),
            CupsError::Status(s) => write!(f, "printing service refused the request (IPP status 0x{s:04x})"),
            CupsError::Missing(what) => write!(f, "printing service reply lacks {what}"),
            CupsError::BadName(n) => write!(f, "invalid printer name {n:?}"),
            CupsError::BadDeviceUri(u) => write!(f, "unsupported device URI {u:?}"),
        }
    }
}

impl std::error::Error for CupsError {}

impl From<IppError> for CupsError {
    fn from(e: IppError) -> CupsError {
        CupsError::Protocol(e)
    }
}

/// Sends one IPP request body and returns the IPP response body.
pub trait Transport {
    fn send(&mut self, body: &[u8]) -> Result<Vec<u8>, CupsError>;
}

/// HTTP/1.1 POST of `application/ipp` to the scheduler (path `/`; the printer is named in `printer-uri`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpTransport {
    pub host: String,
    pub port: u16,
}

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport { host: DEFAULT_HOST.to_string(), port: DEFAULT_PORT }
    }
}

impl Transport for HttpTransport {
    fn send(&mut self, body: &[u8]) -> Result<Vec<u8>, CupsError> {
        let io = |e: std::io::Error| CupsError::Io(e.to_string());
        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).map_err(io)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
        let head = format!(
            "POST / HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.host,
            self.port,
            body.len()
        );
        stream.write_all(head.as_bytes()).map_err(io)?;
        stream.write_all(body).map_err(io)?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(io)?;
        parse_http_response(&raw)
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn header_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let (key, value) = line.split_once(':')?;
    if key.trim().eq_ignore_ascii_case(name) {
        Some(value.trim())
    } else {
        None
    }
}

/// Returns the entity body of a 200 response, de-chunking it when the transfer is chunked.
pub fn parse_http_response(raw: &[u8]) -> Result<Vec<u8>, CupsError> {
    let split = find(raw, b"\r\n\r\n").ok_or_else(|| CupsError::Http("no end of headers".into()))?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| CupsError::Http("headers are not text".into()))?;
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let code = status_line.split_whitespace().nth(1).and_then(|c| c.parse::<u16>().ok());
    match code {
        Some(200) => {}
        Some(other) => return Err(CupsError::Http(format!("HTTP {other}"))),
        None => return Err(CupsError::Http(format!("bad status line {status_line:?}"))),
    }
    let headers: Vec<&str> = lines.collect();
    let chunked = headers
        .iter()
        .any(|l| header_value(l, "transfer-encoding").map_or(false, |v| v.eq_ignore_ascii_case("chunked")));
    if chunked {
        return dechunk(body);
    }
    let length = headers.iter().find_map(|l| header_value(l, "content-length")).and_then(|v| v.parse::<usize>().ok());
    match length {
        Some(n) if n <= body.len() => Ok(body[..n].to_vec()),
        _ => Ok(body.to_vec()),
    }
}

fn dechunk(body: &[u8]) -> Result<Vec<u8>, CupsError> {
    let bad = || CupsError::Http("bad chunked body".into());
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let rest = body.get(pos..).ok_or_else(bad)?;
        let line_end = find(rest, b"\r\n").ok_or_else(bad)?;
        let size_text = std::str::from_utf8(&rest[..line_end]).map_err(|_| bad())?;
        let size_hex = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| bad())?;
        pos += line_end + 2;
        if size == 0 {
            return Ok(out);
        }
        let end = pos.checked_add(size).ok_or_else(bad)?;
        let chunk = body.get(pos..end).ok_or_else(bad)?;
        out.extend_from_slice(chunk);
        pos = end + 2;
    }
}

/// `printer-state` (RFC 8011 section 5.4.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterState {
    Idle,
    Processing,
    Stopped,
    Other(i32),
}

impl PrinterState {
    pub fn from_code(code: i32) -> PrinterState {
        match code {
            3 => PrinterState::Idle,
            4 => PrinterState::Processing,
            5 => PrinterState::Stopped,
            other => PrinterState::Other(other),
        }
    }
}

/// `job-state` (RFC 8011 section 5.3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Pending,
    PendingHeld,
    Processing,
    ProcessingStopped,
    Canceled,
    Aborted,
    Completed,
    Other(i32),
}

impl JobState {
    pub fn from_code(code: i32) -> JobState {
        match code {
            3 => JobState::Pending,
            4 => JobState::PendingHeld,
            5 => JobState::Processing,
            6 => JobState::ProcessingStopped,
            7 => JobState::Canceled,
            8 => JobState::Aborted,
            9 => JobState::Completed,
            other => JobState::Other(other),
        }
    }

    /// True while the job still occupies the queue.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            JobState::Pending | JobState::PendingHeld | JobState::Processing | JobState::ProcessingStopped
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Printer {
    pub name: String,
    pub uri: String,
    pub state: PrinterState,
    pub state_message: String,
    pub accepting_jobs: bool,
    pub is_default: bool,
    pub make_and_model: String,
    pub location: String,
    pub info: String,
    pub device_uri: String,
    pub queued_jobs: i32,
    /// `printer-state-reasons` without `none`: keywords such as `media-jam-error` or `toner-low-warning`.
    pub state_reasons: Vec<String>,
    /// Ink and toner from the `marker-*` attributes.
    pub supplies: Vec<Supply>,
    /// `printer-is-shared`: the queue is offered to other machines.
    pub shared: bool,
    /// A printer pool (class): the queue names it spreads jobs over (`member-names`). Empty for a plain printer.
    pub members: Vec<String>,
    /// True for a pool, even one without members yet.
    pub is_class: bool,
}

/// One ink or toner cartridge.
#[derive(Debug, Clone, PartialEq)]
pub struct Supply {
    pub name: String,
    /// 0 to 100, or `None` when the printer does not report a level (`marker-levels` below zero).
    pub level: Option<u8>,
    /// `#RRGGBB`, or empty when the printer names no colour.
    pub color: String,
}

/// The supplies of a printer group: `marker-names`, `marker-levels` and `marker-colors` side by side.
pub fn supplies_from_group(g: &Group) -> Vec<Supply> {
    let names = g.texts("marker-names");
    let levels = g.ints("marker-levels");
    let colors = g.texts("marker-colors");
    names
        .iter()
        .enumerate()
        .map(|(i, name)| Supply {
            name: name.to_string(),
            level: levels.get(i).copied().filter(|l| (0..=100).contains(l)).map(|l| l as u8),
            color: colors.get(i).filter(|c| c.starts_with('#')).map_or(String::new(), |c| c.to_string()),
        })
        .collect()
}

impl Printer {
    /// Reads a printer-attributes group. A group without `printer-name` is not a printer.
    pub fn from_group(g: &Group, is_default: bool) -> Option<Printer> {
        let name = g.text("printer-name")?.to_string();
        Some(Printer {
            uri: g.text("printer-uri-supported").map(str::to_string).unwrap_or_else(|| printer_uri(&name)),
            name,
            state: PrinterState::from_code(g.int("printer-state").unwrap_or(0)),
            state_message: g.text("printer-state-message").unwrap_or("").to_string(),
            accepting_jobs: g.boolean("printer-is-accepting-jobs").unwrap_or(false),
            is_default,
            make_and_model: g.text("printer-make-and-model").unwrap_or("").to_string(),
            location: g.text("printer-location").unwrap_or("").to_string(),
            info: g.text("printer-info").unwrap_or("").to_string(),
            device_uri: g.text("device-uri").unwrap_or("").to_string(),
            queued_jobs: g.int("queued-job-count").unwrap_or(0),
            state_reasons: g.texts("printer-state-reasons").into_iter().filter(|r| *r != "none").map(str::to_string).collect(),
            supplies: supplies_from_group(g),
            shared: g.boolean("printer-is-shared").unwrap_or(false),
            members: g.texts("member-names").into_iter().map(str::to_string).collect(),
            is_class: false,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: i32,
    pub uri: String,
    pub state: JobState,
    pub name: String,
    pub user: String,
    pub size_kb: i32,
    /// `job-state-reasons` without `none`: `job-hold-until-specified`, `cups-remote-canceled`...
    pub reasons: Vec<String>,
    /// `time-at-creation`: seconds since the Unix epoch, 0 when the scheduler gave none.
    pub created: i32,
    /// `job-impressions`: the pages the job will print, 0 when unknown.
    pub pages: i32,
    /// `job-impressions-completed`: the pages printed so far.
    pub pages_done: i32,
    /// `job-media-progress`: percent printed, `None` when the scheduler reports none.
    pub progress: Option<u8>,
}

impl Job {
    /// Reads a job-attributes group; a group without `job-id` is skipped.
    pub fn from_group(g: &Group) -> Option<Job> {
        Some(Job {
            id: g.int("job-id")?,
            uri: g.text("job-uri").unwrap_or("").to_string(),
            state: JobState::from_code(g.int("job-state").unwrap_or(0)),
            name: g.text("job-name").unwrap_or("").to_string(),
            user: g.text("job-originating-user-name").unwrap_or("").to_string(),
            size_kb: g.int("job-k-octets").unwrap_or(0),
            reasons: g.texts("job-state-reasons").into_iter().filter(|r| *r != "none").map(str::to_string).collect(),
            created: g.int("time-at-creation").unwrap_or(0),
            pages: g.int("job-impressions").unwrap_or(0),
            pages_done: g.int("job-impressions-completed").unwrap_or(0),
            progress: g.int("job-media-progress").filter(|p| (0..=100).contains(p)).map(|p| p as u8),
        })
    }
}

/// `ipp://localhost/printers/NAME`.
pub fn printer_uri(name: &str) -> String {
    format!("ipp://{DEFAULT_HOST}/printers/{name}")
}

/// Names the scheduler accepts: 1 to 127 letters, digits, `-`, `_` or `.`.
pub fn valid_printer_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 127
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Device URIs the driverless path can add: IPP, IPPS, raw socket and USB.
pub fn valid_device_uri(uri: &str) -> bool {
    ["ipp://", "ipps://", "socket://", "usb://"].iter().any(|s| uri.starts_with(s) && uri.len() > s.len())
}

/// A queue name from a device address: its host (or the USB or socket target) with characters the scheduler does
/// not take replaced by hyphens. `taken` names are numbered (`-2`, `-3`) so an existing queue is never replaced.
pub fn queue_name(device_uri: &str, taken: &[String]) -> String {
    let rest = device_uri.split_once("://").map_or(device_uri, |(_, r)| r);
    let host: String = rest
        .split(|c| c == '/' || c == ':')
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(60)
        .collect();
    let base = if host.trim_matches('-').is_empty() { "Printer".to_string() } else { host.trim_matches('-').to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Adds a queue for the device at `device_uri` (driverless) under a name no existing queue uses, and returns that
/// name. The address is trimmed first; an address the scheduler does not take is refused before any request.
pub fn add_device<T: Transport>(client: &mut Client<T>, device_uri: &str) -> Result<String, CupsError> {
    let device = device_uri.trim();
    if !valid_device_uri(device) {
        return Err(CupsError::BadDeviceUri(device.to_string()));
    }
    let taken: Vec<String> = client.printers()?.into_iter().map(|p| p.name).collect();
    let name = queue_name(device, &taken);
    client.add_driverless_printer(&name, device)?;
    Ok(name)
}

/// Joins template entries that share a name into one attribute with several values, in order (IPP multi-valued
/// attributes such as one `page-ranges` value per range). Names keep the order they first appear in.
pub fn merge_attributes(template: Vec<(String, Value)>) -> Vec<(String, Vec<Value>)> {
    let mut merged: Vec<(String, Vec<Value>)> = Vec::new();
    for (name, value) in template {
        let index = merged.iter().position(|(n, _)| *n == name);
        match index {
            Some(i) => merged[i].1.push(value),
            None => merged.push((name, vec![value])),
        }
    }
    merged
}

fn base(operation: u16) -> Request {
    Request::new(operation)
        .with(group::OPERATION, "attributes-charset", vec![Value::charset("utf-8")])
        .with(group::OPERATION, "attributes-natural-language", vec![Value::natural_language("en")])
}

fn keywords(list: &[&str]) -> Vec<Value> {
    list.iter().map(|s| Value::keyword(s)).collect()
}

fn target(name: &str) -> Result<String, CupsError> {
    if valid_printer_name(name) {
        Ok(printer_uri(name))
    } else {
        Err(CupsError::BadName(name.to_string()))
    }
}

pub struct Client<T: Transport> {
    transport: T,
    next_id: u32,
}

impl Client<HttpTransport> {
    /// A client for the local scheduler.
    pub fn local() -> Client<HttpTransport> {
        Client::new(HttpTransport::default())
    }
}

impl<T: Transport> Client<T> {
    pub fn new(transport: T) -> Client<T> {
        Client { transport, next_id: 1 }
    }

    fn exchange(&mut self, mut req: Request) -> Result<Response, CupsError> {
        req.request_id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let raw = self.transport.send(&req.encode())?;
        let resp = Response::decode(&raw)?;
        if resp.request_id != req.request_id {
            return Err(CupsError::Protocol(IppError::Malformed("request id mismatch")));
        }
        if resp.is_success() {
            Ok(resp)
        } else {
            Err(CupsError::Status(resp.status))
        }
    }

    /// The default printer's name, or `None` when the scheduler has no default.
    pub fn default_printer(&mut self) -> Result<Option<String>, CupsError> {
        let req = base(op::CUPS_GET_DEFAULT)
            .with(group::OPERATION, "requested-attributes", keywords(&["printer-name"]));
        match self.exchange(req) {
            Ok(resp) => Ok(resp
                .groups_with(group::PRINTER)
                .find_map(|g| g.text("printer-name").map(str::to_string))),
            Err(CupsError::Status(s)) if s == ipp::STATUS_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Every printer and queue the scheduler knows, with the default one flagged.
    pub fn printers(&mut self) -> Result<Vec<Printer>, CupsError> {
        let default = self.default_printer()?;
        let req = base(op::CUPS_GET_PRINTERS)
            .with(group::OPERATION, "requested-attributes", keywords(PRINTER_ATTRIBUTES));
        let resp = self.exchange(req)?;
        let mut printers: Vec<Printer> = resp
            .groups_with(group::PRINTER)
            .filter_map(|g| Printer::from_group(g, default.as_deref() == g.text("printer-name")))
            .collect();
        // pools: a scheduler that lists them with the printers already has them; the rest are added here
        for pool in self.classes(default.as_deref())? {
            match printers.iter_mut().find(|p| p.name == pool.name) {
                Some(existing) => *existing = pool,
                None => printers.push(pool),
            }
        }
        Ok(printers)
    }

    /// The printer pools (classes) of the scheduler (CUPS-Get-Classes), flagged `is_class`.
    pub fn classes(&mut self, default: Option<&str>) -> Result<Vec<Printer>, CupsError> {
        let req = base(op::CUPS_GET_CLASSES)
            .with(group::OPERATION, "requested-attributes", keywords(PRINTER_ATTRIBUTES));
        match self.exchange(req) {
            Ok(resp) => Ok(resp
                .groups_with(group::PRINTER)
                .filter_map(|g| Printer::from_group(g, default == g.text("printer-name")))
                .map(|mut p| {
                    p.is_class = true;
                    p
                })
                .collect()),
            // a scheduler without pools answers not-found
            Err(CupsError::Status(s)) if s == ipp::STATUS_NOT_FOUND => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Removes a printer pool (CUPS-Delete-Class); its member printers stay.
    pub fn delete_class(&mut self, name: &str) -> Result<(), CupsError> {
        self.printer_op(op::CUPS_DELETE_CLASS, name)
    }

    /// Sets the description and location a person sees for the queue (CUPS-Add-Modify-Printer).
    pub fn set_printer_details(&mut self, printer: &str, info: &str, location: &str) -> Result<(), CupsError> {
        let req = base(op::CUPS_ADD_MODIFY_PRINTER)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::PRINTER, "printer-info", vec![Value::text(info)])
            .with(group::PRINTER, "printer-location", vec![Value::text(location)]);
        self.exchange(req).map(|_| ())
    }

    /// Offers the queue to other machines, or stops offering it (`printer-is-shared`).
    pub fn set_shared(&mut self, printer: &str, shared: bool) -> Result<(), CupsError> {
        let req = base(op::CUPS_ADD_MODIFY_PRINTER)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::PRINTER, "printer-is-shared", vec![Value::Boolean(shared)]);
        self.exchange(req).map(|_| ())
    }

    /// Capabilities of one printer (Get-Printer-Attributes): paper list, duplex, colour modes and status.
    pub fn get_printer_attributes(&mut self, printer: &str) -> Result<crate::printer_caps::PrinterCapabilities, CupsError> {
        let req = base(op::GET_PRINTER_ATTRIBUTES)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::OPERATION, "requested-attributes", keywords(CAPABILITY_ATTRIBUTES));
        let resp = self.exchange(req)?;
        let g = resp.groups_with(group::PRINTER).next().ok_or(CupsError::Missing("printer attributes"))?;
        Ok(crate::printer_caps::PrinterCapabilities::from_group(g))
    }

    /// Jobs still in the queue of `printer` (completed and canceled jobs are left out).
    pub fn jobs(&mut self, printer: &str) -> Result<Vec<Job>, CupsError> {
        let req = base(op::GET_JOBS)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::OPERATION, "which-jobs", vec![Value::keyword("not-completed")])
            .with(group::OPERATION, "requested-attributes", keywords(JOB_ATTRIBUTES));
        let resp = self.exchange(req)?;
        Ok(resp.groups_with(group::JOB).filter_map(Job::from_group).collect())
    }

    pub fn cancel_job(&mut self, printer: &str, job_id: i32) -> Result<(), CupsError> {
        let req = base(op::CANCEL_JOB)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::OPERATION, "job-id", vec![Value::Integer(job_id)]);
        self.exchange(req).map(|_| ())
    }

    /// Holds a queued job so it is not printed until released (Hold-Job).
    pub fn hold_job(&mut self, printer: &str, job_id: i32) -> Result<(), CupsError> {
        self.job_op(op::HOLD_JOB, printer, job_id)
    }

    /// Lets a held job print (Release-Job).
    pub fn release_job(&mut self, printer: &str, job_id: i32) -> Result<(), CupsError> {
        self.job_op(op::RELEASE_JOB, printer, job_id)
    }

    fn job_op(&mut self, operation: u16, printer: &str, job_id: i32) -> Result<(), CupsError> {
        let req = base(operation)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::OPERATION, "job-id", vec![Value::Integer(job_id)]);
        self.exchange(req).map(|_| ())
    }

    pub fn pause_printer(&mut self, printer: &str) -> Result<(), CupsError> {
        self.printer_op(op::PAUSE_PRINTER, printer)
    }

    pub fn resume_printer(&mut self, printer: &str) -> Result<(), CupsError> {
        self.printer_op(op::RESUME_PRINTER, printer)
    }

    pub fn set_default(&mut self, printer: &str) -> Result<(), CupsError> {
        self.printer_op(op::CUPS_SET_DEFAULT, printer)
    }

    pub fn delete_printer(&mut self, printer: &str) -> Result<(), CupsError> {
        self.printer_op(op::CUPS_DELETE_PRINTER, printer)
    }

    fn printer_op(&mut self, operation: u16, printer: &str) -> Result<(), CupsError> {
        let req = base(operation).with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)]);
        self.exchange(req).map(|_| ())
    }

    /// Adds a queue for a network or USB device that speaks IPP Everywhere, with the generic driverless PPD.
    pub fn add_driverless_printer(&mut self, name: &str, device_uri: &str) -> Result<(), CupsError> {
        let uri = target(name)?;
        if !valid_device_uri(device_uri) {
            return Err(CupsError::BadDeviceUri(device_uri.to_string()));
        }
        let req = base(op::CUPS_ADD_MODIFY_PRINTER)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&uri)])
            .with(group::PRINTER, "device-uri", vec![Value::uri(device_uri)])
            .with(group::PRINTER, "ppd-name", vec![Value::keyword("everywhere")])
            .with(group::PRINTER, "printer-is-accepting-jobs", vec![Value::Boolean(true)]);
        self.exchange(req).map(|_| ())
    }

    /// Submits a document (Print-Job) and returns the new job id.
    pub fn print_document(
        &mut self,
        printer: &str,
        job_name: &str,
        user: &str,
        mime: &str,
        data: Vec<u8>,
    ) -> Result<i32, CupsError> {
        self.print_document_with(printer, job_name, user, mime, data, Vec::new())
    }

    /// Submits a document with job-template attributes (copies, media, orientation, page ranges, plug-in options). Each
    /// `(name, value)` pair is one attribute of the job group. Returns the new job id.
    pub fn print_document_with(
        &mut self,
        printer: &str,
        job_name: &str,
        user: &str,
        mime: &str,
        data: Vec<u8>,
        template: Vec<(String, Value)>,
    ) -> Result<i32, CupsError> {
        let mut req = base(op::PRINT_JOB)
            .with(group::OPERATION, "printer-uri", vec![Value::uri(&target(printer)?)])
            .with(group::OPERATION, "requesting-user-name", vec![Value::name(user)])
            .with(group::OPERATION, "job-name", vec![Value::name(job_name)])
            .with(group::OPERATION, "document-format", vec![Value::mime(mime)]);
        for (name, values) in merge_attributes(template) {
            req = req.with(group::JOB, &name, values);
        }
        let resp = self.exchange(req.document(data))?;
        // bind the id first: the iterator borrows `resp`, which a tail expression would outlive
        let id = resp.groups_with(group::JOB).find_map(|g| g.int("job-id"));
        id.ok_or(CupsError::Missing("job-id"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::{tag, Attribute};
    use std::collections::VecDeque;

    /// Replies in order and records what was sent. Nothing touches the network.
    struct Canned {
        replies: VecDeque<Vec<u8>>,
        sent: Vec<Vec<u8>>,
    }

    impl Canned {
        fn new(replies: Vec<Vec<u8>>) -> Canned {
            Canned { replies: replies.into(), sent: Vec::new() }
        }
    }

    impl Transport for Canned {
        fn send(&mut self, body: &[u8]) -> Result<Vec<u8>, CupsError> {
            self.sent.push(body.to_vec());
            self.replies.pop_front().ok_or(CupsError::Io("no canned reply".into()))
        }
    }

    fn reply(status: u16, id: u32, groups: Vec<Group>) -> Vec<u8> {
        let mut r = Request::new(status);
        r.request_id = id;
        r.groups = groups;
        r.encode()
    }

    fn attr(name: &str, v: Value) -> Attribute {
        Attribute { name: name.to_string(), values: vec![v] }
    }

    fn printer_group(name: &str, state: i32, accepting: bool) -> Group {
        Group {
            tag: group::PRINTER,
            attributes: vec![
                attr("printer-name", Value::name(name)),
                attr("printer-state", Value::Enum(state)),
                attr("printer-is-accepting-jobs", Value::Boolean(accepting)),
                attr("printer-uri-supported", Value::uri(&printer_uri(name))),
                attr("queued-job-count", Value::Integer(2)),
            ],
        }
    }

    fn default_reply(id: u32, name: &str) -> Vec<u8> {
        reply(0, id, vec![Group { tag: group::PRINTER, attributes: vec![attr("printer-name", Value::name(name))] }])
    }

    #[test]
    fn printers_are_listed_with_the_default_flagged() {
        let canned = Canned::new(vec![
            default_reply(1, "Office"),
            reply(0, 2, vec![printer_group("Office", 3, true), printer_group("Lab", 5, false)]),
            reply(ipp::STATUS_NOT_FOUND, 3, vec![]),
        ]);
        let mut client = Client::new(canned);
        let printers = client.printers().unwrap();
        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].name, "Office");
        assert!(printers[0].is_default);
        assert_eq!(printers[0].state, PrinterState::Idle);
        assert!(printers[0].accepting_jobs);
        assert_eq!(printers[0].queued_jobs, 2);
        assert!(!printers[1].is_default);
        assert_eq!(printers[1].state, PrinterState::Stopped);
        assert!(!printers[1].accepting_jobs);
        assert_eq!(printers[1].uri, "ipp://localhost/printers/Lab");
    }

    #[test]
    fn hold_and_release_send_their_operation_with_the_job_id() {
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![]), reply(0, 2, vec![])]));
        client.hold_job("Office", 7).unwrap();
        client.release_job("Office", 7).unwrap();
        let sent = &client.transport.sent;
        // the operation id is the big-endian u16 after the 2-byte version
        assert_eq!(&sent[0][2..4], &op::HOLD_JOB.to_be_bytes());
        assert_eq!(&sent[1][2..4], &op::RELEASE_JOB.to_be_bytes());
        assert!(sent[0].windows(4).any(|w| w == 7i32.to_be_bytes().as_slice()));
    }

    #[test]
    fn a_bad_printer_name_is_refused_before_any_request() {
        let mut client = Client::new(Canned::new(vec![]));
        assert_eq!(client.hold_job("bad name", 1), Err(CupsError::BadName("bad name".into())));
        assert!(client.transport.sent.is_empty());
    }

    #[test]
    fn no_default_is_none_not_an_error() {
        let mut client = Client::new(Canned::new(vec![reply(ipp::STATUS_NOT_FOUND, 1, vec![])]));
        assert_eq!(client.default_printer().unwrap(), None);
    }

    #[test]
    fn refused_request_reports_the_status() {
        let mut client = Client::new(Canned::new(vec![reply(0x0500, 1, vec![])]));
        assert_eq!(client.set_default("Lab"), Err(CupsError::Status(0x0500)));
    }

    #[test]
    fn request_id_mismatch_is_reported() {
        let mut client = Client::new(Canned::new(vec![reply(0, 9, vec![])]));
        assert_eq!(client.default_printer(), Err(CupsError::Protocol(IppError::Malformed("request id mismatch"))));
    }

    #[test]
    fn jobs_map_state_and_skip_groups_without_id() {
        let job = Group {
            tag: group::JOB,
            attributes: vec![
                attr("job-id", Value::Integer(4)),
                attr("job-state", Value::Enum(5)),
                attr("job-name", Value::name("report.pdf")),
                attr("job-originating-user-name", Value::name("olibuijr")),
                attr("job-k-octets", Value::Integer(120)),
            ],
        };
        let empty = Group { tag: group::JOB, attributes: vec![attr("job-name", Value::name("x"))] };
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![job, empty])]));
        let jobs = client.jobs("Lab").unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, 4);
        assert_eq!(jobs[0].state, JobState::Processing);
        assert!(jobs[0].state.is_active());
        assert_eq!(jobs[0].name, "report.pdf");
        assert_eq!(jobs[0].user, "olibuijr");
        assert_eq!(jobs[0].size_kb, 120);
    }

    #[test]
    fn print_document_sends_the_bytes_after_the_attributes() {
        let job = Group { tag: group::JOB, attributes: vec![attr("job-id", Value::Integer(9))] };
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![job])]));
        let id = client.print_document("Lab", "report.pdf", "olibuijr", "application/pdf", b"%PDF-1.4".to_vec());
        assert_eq!(id, Ok(9));
        let sent = &client.transport.sent[0];
        assert!(sent.ends_with(b"%PDF-1.4"));
        assert!(find(sent, b"application/pdf").is_some());
        assert!(find(sent, b"ipp://localhost/printers/Lab").is_some());
    }

    #[test]
    fn print_document_with_sends_the_job_template_in_the_job_group() {
        let job = Group { tag: group::JOB, attributes: vec![attr("job-id", Value::Integer(4))] };
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![job])]));
        let template = vec![("copies".to_string(), Value::Integer(2)), ("orientation-requested".to_string(), Value::Enum(4))];
        let id = client.print_document_with("Lab", "a.pdf", "olibuijr", "application/pdf", b"%PDF".to_vec(), template);
        assert_eq!(id, Ok(4));
        let sent = &client.transport.sent[0];
        assert!(find(sent, b"copies").is_some());
        assert!(find(sent, b"orientation-requested").is_some());
        assert!(sent.ends_with(b"%PDF"));
    }

    #[test]
    fn merge_attributes_keeps_one_attribute_per_name_with_its_values_in_order() {
        let merged = merge_attributes(vec![
            ("page-ranges".to_string(), Value::Integer(1)),
            ("copies".to_string(), Value::Integer(2)),
            ("page-ranges".to_string(), Value::Integer(3)),
        ]);
        assert_eq!(
            merged,
            vec![
                ("page-ranges".to_string(), vec![Value::Integer(1), Value::Integer(3)]),
                ("copies".to_string(), vec![Value::Integer(2)]),
            ]
        );
    }

    #[test]
    fn invalid_names_and_device_uris_are_refused_before_sending() {
        let mut client = Client::new(Canned::new(vec![]));
        assert_eq!(client.cancel_job("bad name!", 1), Err(CupsError::BadName("bad name!".into())));
        assert_eq!(
            client.add_driverless_printer("Office", "http://printer.local/"),
            Err(CupsError::BadDeviceUri("http://printer.local/".into()))
        );
        assert!(client.transport.sent.is_empty());
    }

    #[test]
    fn driverless_add_uses_the_everywhere_ppd() {
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![])]));
        client.add_driverless_printer("Office", "ipp://192.168.1.50/ipp/print").unwrap();
        let sent = &client.transport.sent[0];
        assert!(find(sent, b"everywhere").is_some());
        assert!(find(sent, b"ipp://192.168.1.50/ipp/print").is_some());
    }

    #[test]
    fn queue_names_come_from_the_host_and_never_repeat() {
        assert_eq!(queue_name("ipp://printer.local/ipp/print", &[]), "printer-local");
        assert_eq!(queue_name("socket://192.168.1.20:9100", &[]), "192-168-1-20");
        assert_eq!(queue_name("usb://HP/LaserJet", &[]), "HP");
        assert_eq!(queue_name("usb:///", &[]), "Printer");
        let taken = vec!["HP".to_string(), "HP-2".to_string()];
        assert_eq!(queue_name("usb://HP/LaserJet", &taken), "HP-3");
    }

    #[test]
    fn add_device_lists_the_queues_then_adds_under_a_free_name() {
        let canned = Canned::new(vec![
            default_reply(1, "HP"),
            reply(0, 2, vec![printer_group("HP", 3, true)]),
            reply(ipp::STATUS_NOT_FOUND, 3, vec![]),
            reply(0, 4, vec![]),
        ]);
        let mut client = Client::new(canned);
        let name = add_device(&mut client, "  usb://HP/LaserJet  ").unwrap();
        assert_eq!(name, "HP-2");
        let sent = &client.transport.sent[3];
        assert!(find(sent, b"usb://HP/LaserJet").is_some(), "the trimmed address is sent");
        assert!(find(sent, b"HP-2").is_some());
    }

    #[test]
    fn add_device_refuses_an_address_the_scheduler_does_not_take_without_a_request() {
        let mut client = Client::new(Canned::new(vec![]));
        assert_eq!(add_device(&mut client, "ftp://x"), Err(CupsError::BadDeviceUri("ftp://x".into())));
        assert!(client.transport.sent.is_empty());
    }

    #[test]
    fn supplies_pair_names_levels_and_colours() {
        let g = Group {
            tag: group::PRINTER,
            attributes: vec![
                Attribute { name: "marker-names".into(), values: vec![Value::name("Black"), Value::name("Cyan"), Value::name("Waste")] },
                Attribute { name: "marker-levels".into(), values: vec![Value::Integer(80), Value::Integer(-1), Value::Integer(100)] },
                Attribute { name: "marker-colors".into(), values: vec![Value::name("#000000"), Value::name("#00FFFF"), Value::name("none")] },
            ],
        };
        let s = supplies_from_group(&g);
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].name.as_str(), s[0].level, s[0].color.as_str()), ("Black", Some(80), "#000000"));
        assert_eq!(s[1].level, None, "a negative level is unknown");
        assert_eq!(s[2].color, "");
    }

    #[test]
    fn printer_reasons_and_job_progress_are_read() {
        let mut g = printer_group("Lab", 3, true);
        g.attributes.push(Attribute { name: "printer-state-reasons".into(), values: vec![Value::keyword("media-jam-error"), Value::keyword("none")] });
        g.attributes.push(attr("printer-is-shared", Value::Boolean(true)));
        let p = Printer::from_group(&g, false).unwrap();
        assert_eq!(p.state_reasons, vec!["media-jam-error".to_string()]);
        assert!(p.shared);
        assert!(!p.is_class);
        let j = Group {
            tag: group::JOB,
            attributes: vec![
                attr("job-id", Value::Integer(2)),
                attr("time-at-creation", Value::Integer(1_700_000_000)),
                attr("job-impressions", Value::Integer(10)),
                attr("job-impressions-completed", Value::Integer(4)),
                attr("job-media-progress", Value::Integer(40)),
                attr("job-state-reasons", Value::keyword("job-hold-until-specified")),
            ],
        };
        let job = Job::from_group(&j).unwrap();
        assert_eq!((job.created, job.pages, job.pages_done, job.progress), (1_700_000_000, 10, 4, Some(40)));
        assert_eq!(job.reasons, vec!["job-hold-until-specified".to_string()]);
    }

    #[test]
    fn classes_are_flagged_and_replace_the_same_named_printer() {
        let mut pool = printer_group("Floor", 3, true);
        pool.attributes.push(Attribute { name: "member-names".into(), values: vec![Value::name("Office"), Value::name("Lab")] });
        let canned = Canned::new(vec![
            default_reply(1, "Office"),
            reply(0, 2, vec![printer_group("Office", 3, true), printer_group("Floor", 3, true)]),
            reply(0, 3, vec![pool]),
        ]);
        let printers = Client::new(canned).printers().unwrap();
        assert_eq!(printers.len(), 2);
        let floor = printers.iter().find(|p| p.name == "Floor").unwrap();
        assert!(floor.is_class);
        assert_eq!(floor.members, vec!["Office".to_string(), "Lab".to_string()]);
        assert!(!printers.iter().find(|p| p.name == "Office").unwrap().is_class);
    }

    #[test]
    fn details_and_sharing_send_their_attributes() {
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![]), reply(0, 2, vec![]), reply(0, 3, vec![])]));
        client.set_printer_details("Office", "Front desk", "Floor 2").unwrap();
        client.set_shared("Office", false).unwrap();
        client.delete_class("Floor").unwrap();
        let sent = &client.transport.sent;
        assert!(find(&sent[0], b"printer-location").is_some() && find(&sent[0], b"Floor 2").is_some());
        assert!(find(&sent[1], b"printer-is-shared").is_some());
        assert_eq!(&sent[2][2..4], &op::CUPS_DELETE_CLASS.to_be_bytes());
    }

    #[test]
    fn valid_names() {
        assert!(valid_printer_name("Office_2.floor-1"));
        assert!(!valid_printer_name(""));
        assert!(!valid_printer_name("a/b"));
        assert!(!valid_printer_name(&"a".repeat(128)));
    }

    #[test]
    fn job_state_codes() {
        assert_eq!(JobState::from_code(7), JobState::Canceled);
        assert_eq!(JobState::from_code(9), JobState::Completed);
        assert!(!JobState::from_code(9).is_active());
        assert_eq!(JobState::from_code(42), JobState::Other(42));
    }

    #[test]
    fn plain_http_body_honours_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: 3\r\n\r\nabcdef";
        assert_eq!(parse_http_response(raw), Ok(b"abc".to_vec()));
    }

    #[test]
    fn chunked_http_body_is_joined() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n4;ext=1\r\ndefg\r\n0\r\n\r\n";
        assert_eq!(parse_http_response(raw), Ok(b"abcdefg".to_vec()));
    }

    #[test]
    fn http_errors_are_reported() {
        assert!(matches!(parse_http_response(b"HTTP/1.1 500 Oops\r\n\r\n"), Err(CupsError::Http(_))));
        assert!(matches!(parse_http_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"), Err(CupsError::Http(_))));
        assert!(matches!(parse_http_response(b"no headers here"), Err(CupsError::Http(_))));
    }

    #[test]
    fn attribute_helper_builds_a_name_value_pair() {
        let a = attr("job-id", Value::Integer(1));
        assert_eq!(a.name, "job-id");
        assert_eq!(a.values, vec![Value::Integer(1)]);
        assert_eq!(tag::INTEGER, 0x21);
    }

    #[test]
    fn printer_attributes_reply_becomes_capabilities() {
        let g = Group {
            tag: group::PRINTER,
            attributes: vec![
                attr("media-supported", Value::keyword("iso_a4_210x297mm")),
                attr("sides-supported", Value::keyword("two-sided-long-edge")),
                attr("printer-state", Value::Enum(3)),
            ],
        };
        let mut client = Client::new(Canned::new(vec![reply(0, 1, vec![g])]));
        let caps = client.get_printer_attributes("Office").unwrap();
        assert_eq!(caps.papers.len(), 1);
        assert!(caps.supports_duplex());
        assert_eq!(caps.status.state, PrinterState::Idle);
        assert!(client.get_printer_attributes("bad name").is_err());
    }
}
