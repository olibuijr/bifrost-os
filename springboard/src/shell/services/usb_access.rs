//! The `usb` service: raw access to one USB or raw HID device, for one app, with the person's consent.
//!
//! Nothing is open by default and no device node ever reaches an app sandbox. An app that lists the `usb` (or `hid`)
//! manifest permission can only ask. The person picks the device on the permission sheet (Allow Once, Always Allow,
//! Don't Allow), and only that device is granted to that app (`bifrost_core::usb_grants`). Then the shell opens the
//! node itself and runs the app's requests through the rules of `bifrost_core::usb_session` (USB) and
//! `bifrost_core::hidraw` (HID), on a worker thread per open device, so the shell thread never waits on a device.
//!
//! An app sees only devices it holds a grant for: `devices` lists nothing else, a plug or unplug of an ungranted
//! device is silent, and `request` reports how many devices matched but not which. Devices the system drives itself
//! are never offered for raw USB (audio, HID, storage, video, hubs, smart cards, wireless controllers), and keyboards,
//! keypads and security keys are never offered for raw HID.
//!
//! Methods (params are a JSON object; `kind` is `usb` (default) or `hid`, and needs the matching manifest permission):
//! - `devices {kind}` returns the granted, connected devices: `{id, name, vendor_id, product_id, kind, ...}`.
//! - `request {kind, filters}` asks the person to pick a device. `filters` is a list of `{vendor_id, product_id, class,
//!   usage_page, usage}` (all optional; none means any eligible device). Only the app in front may ask, and an app is
//!   asked about a device at most once per run. Returns `{offered, asked}`. The answer arrives as an `access` event.
//! - `revoke {id}` takes the grant back and closes the device.
//! - `open {id}` opens a granted device and returns `{handle, ...descriptors}` (USB) or `{handle, descriptor, usages}`
//!   (HID). A device is open for one app at a time; an app has at most [`MAX_OPEN_PER_APP`] open.
//! - `close {handle}`.
//! - USB, on a handle: `claim {handle, interface}`, `release`, `alternate {handle, interface, alternate}`,
//!   `read {handle, endpoint, length, timeout}`, `write {handle, endpoint, data, timeout}`,
//!   `control {handle, direction, kind, recipient, request, value, index, length, data, timeout}`,
//!   `clear_halt {handle, endpoint}`, `reset {handle}`.
//! - HID, on a handle: `hid_write {handle, data}`, `hid_get_feature {handle, report_id, length}`,
//!   `hid_set_feature {handle, data}`. Input reports arrive as `report` events.
//!
//! Every request on a handle returns `{token}` at once and finishes later as a `done` event `{handle, token, op, ok, data,
//! count, error, failure}`; `data` is hex, `op` names the request and `failure` classes an error (`denied`, `timeout`,
//! `stall`, `disconnected`, `busy`, `overflow`, `other`). At most [`MAX_INFLIGHT`] requests wait per handle.
//!
//! Events (each to one app): `access {id, granted}`, `connected {device}` and `disconnected {id}` for devices the app
//! holds a grant for, `done`, `report {handle, data}`, `closed {handle, reason}`.
//!
//! Allow Once ends when the app quits or the device is unplugged; Always Allow is saved in `usb-app-grants.txt`.
use super::gamepad_service::front_app;
use super::privacy_permissions::{self, Grant, Kind};
use super::{Event, Shell, SystemService};
use bifrost_core::accessory_manager::{Accessory, AccessoryManager};
use bifrost_core::hidraw::{self, HidrawDevice, HidrawInfo};
use bifrost_core::json::Json;
use bifrost_core::plist::{hex_decode, hex_encode};
use bifrost_core::usb_bus_source;
use bifrost_core::usb_descriptors::{ConfigurationDescriptor, DeviceDescriptor, Direction, Recipient, RequestKind, SetupPacket, TransferType};
use bifrost_core::usb_grants::{self, candidates_for, Candidate, Filter, Grants, Scope, MAX_CANDIDATES};
use bifrost_core::usb_host::{self, UsbHostDevice};
use bifrost_core::usb_session::{Session, MAX_BROKER_BYTES};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// The file in the shell data folder that keeps the Always Allow grants.
pub const STORE_FILE: &str = "usb-app-grants.txt";
/// Seconds between reads of the buses.
const SCAN_EVERY: f64 = 1.0;
/// The most devices one app has open at once.
pub const MAX_OPEN_PER_APP: usize = 8;
/// The most requests waiting on one handle.
pub const MAX_INFLIGHT: usize = 16;
/// The most input reports queued per HID handle before the oldest unread ones are dropped.
const MAX_QUEUED_REPORTS: usize = 64;
/// The most filters one request carries.
const MAX_FILTERS: usize = 8;

thread_local! {
    /// The grants of every app. The shell thread owns them: services, sheets and Settings all run on it.
    static GRANTS: RefCell<Grants> = RefCell::new(Grants::new());
    /// The connected devices the last scan found, USB and HID.
    static LIVE: RefCell<Vec<Candidate>> = RefCell::new(Vec::new());
    /// Sheet answers not yet told to their app.
    static ANSWERS: RefCell<Vec<(String, String, bool)>> = RefCell::new(Vec::new());
    /// `(app, device key)` pairs the person was already asked about in this run of the app.
    static ASKED: RefCell<BTreeSet<(String, String)>> = RefCell::new(BTreeSet::new());
    static DIRTY: Cell<bool> = Cell::new(false);
}

/// Records the person's answer to a device sheet: Allow Once and Don't Allow, or (`WhileUsing`) Always Allow. Returns
/// whether the app now holds a grant.
pub fn answer(app: &str, key: &str, grant: Grant) -> bool {
    let granted = match grant {
        Grant::Deny => false,
        Grant::Once => GRANTS.with(|g| g.borrow_mut().grant(app, key, Scope::Once)),
        Grant::WhileUsing => {
            let ok = GRANTS.with(|g| g.borrow_mut().grant(app, key, Scope::Always));
            if ok {
                DIRTY.with(|d| d.set(true));
            }
            ok
        }
    };
    ANSWERS.with(|a| a.borrow_mut().push((app.to_string(), key.to_string(), granted)));
    granted
}

/// Whether the sheet for `key` is still worth showing: the device is connected and the app holds no grant for it.
pub fn still_asked(app: &str, key: &str) -> bool {
    LIVE.with(|l| l.borrow().iter().any(|c| c.key == key)) && !GRANTS.with(|g| g.borrow().is_granted(app, key))
}

/// The name the sheet shows for a device key; the key itself when the device is gone.
pub fn label_of(key: &str) -> String {
    LIVE.with(|l| l.borrow().iter().find(|c| c.key == key).map(|c| c.label())).unwrap_or_else(|| key.to_string())
}

/// What Settings shows for an app: the number of devices it holds a grant for.
pub fn grant_count(app: &str) -> usize {
    GRANTS.with(|g| g.borrow().count(app))
}

/// Takes every device grant of `app` back (Settings, or the app was removed).
pub fn revoke_app(app: &str) {
    GRANTS.with(|g| g.borrow_mut().revoke_app(app));
    ASKED.with(|a| a.borrow_mut().retain(|(x, _)| x != app));
    DIRTY.with(|d| d.set(true));
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn num(n: impl Into<f64>) -> Json {
    Json::Num(n.into())
}

fn text(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}

/// The `kind` parameter: `true` for HID. Anything but `usb` and `hid` is an error.
fn is_hid(params: &Json) -> Result<bool, String> {
    match params.get("kind").and_then(Json::as_str) {
        None | Some("usb") => Ok(false),
        Some("hid") => Ok(true),
        Some(other) => Err(format!("kind must be usb or hid, not {other}")),
    }
}

/// A number parameter that fits `max`. `None` when the key is absent, an error when it is out of range.
fn bounded(params: &Json, key: &str, max: u64) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if n <= max => Ok(Some(n)),
            _ => Err(format!("{key} must be a number from 0 to {max}")),
        },
    }
}

fn required(params: &Json, key: &str, max: u64) -> Result<u64, String> {
    bounded(params, key, max)?.ok_or_else(|| format!("{key} is required"))
}

/// The `filters` parameter. At most [`MAX_FILTERS`] entries; an entry that is not an object is an error.
pub fn parse_filters(value: Option<&Json>) -> Result<Vec<Filter>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    if matches!(value, Json::Null) {
        return Ok(Vec::new());
    }
    let list = value.as_array().ok_or("filters must be a list")?;
    if list.len() > MAX_FILTERS {
        return Err(format!("at most {MAX_FILTERS} filters"));
    }
    list.iter()
        .map(|f| {
            if !matches!(f, Json::Obj(_)) {
                return Err("a filter must be an object".to_string());
            }
            Ok(Filter {
                vendor_id: bounded(f, "vendor_id", 0xffff)?.map(|n| n as u16),
                product_id: bounded(f, "product_id", 0xffff)?.map(|n| n as u16),
                class: bounded(f, "class", 0xff)?.map(|n| n as u8),
                usage_page: bounded(f, "usage_page", 0xffff)?.map(|n| n as u16),
                usage: bounded(f, "usage", 0xffff)?.map(|n| n as u16),
            })
        })
        .collect()
}

/// The control request of a `control` call. Standard requests are parsed but refused later by the session.
pub fn parse_setup(params: &Json) -> Result<SetupPacket, String> {
    let direction = match params.get("direction").and_then(Json::as_str) {
        Some("in") => Direction::DeviceToHost,
        Some("out") => Direction::HostToDevice,
        _ => return Err("direction must be in or out".into()),
    };
    let kind = match params.get("kind").and_then(Json::as_str) {
        Some("class") => RequestKind::Class,
        Some("vendor") => RequestKind::Vendor,
        Some("standard") => RequestKind::Standard,
        _ => return Err("kind must be class or vendor".into()),
    };
    let recipient = match params.get("recipient").and_then(Json::as_str) {
        None | Some("device") => Recipient::Device,
        Some("interface") => Recipient::Interface,
        Some("endpoint") => Recipient::Endpoint,
        _ => return Err("recipient must be device, interface or endpoint".into()),
    };
    Ok(SetupPacket {
        direction,
        kind,
        recipient,
        request: required(params, "request", 0xff)? as u8,
        value: bounded(params, "value", 0xffff)?.unwrap_or(0) as u16,
        index: bounded(params, "index", 0xffff)?.unwrap_or(0) as u16,
        length: bounded(params, "length", 0xffff)?.unwrap_or(0) as u16,
    })
}

/// The `data` parameter as bytes: hex text, at most [`MAX_BROKER_BYTES`] bytes. Absent is empty.
fn data_param(params: &Json) -> Result<Vec<u8>, String> {
    let Some(v) = params.get("data") else { return Ok(Vec::new()) };
    let s = v.as_str().ok_or("data must be hex text")?;
    let bytes = hex_decode(s).ok_or("data is not valid hex")?;
    if bytes.len() > MAX_BROKER_BYTES {
        return Err(format!("data is longer than {MAX_BROKER_BYTES} bytes"));
    }
    Ok(bytes)
}

fn candidate_json(c: &Candidate) -> Json {
    let mut pairs = vec![
        ("id", text(c.key.clone())),
        ("name", text(c.label())),
        ("vendor_id", num(c.vendor_id)),
        ("product_id", num(c.product_id)),
        ("kind", text(if c.hid { "hid" } else { "usb" })),
    ];
    if c.hid {
        pairs.push(("usages", Json::Arr(c.usages.iter().map(|&(p, u)| Json::Arr(vec![num(p), num(u)])).collect())));
    } else {
        pairs.push(("device_class", num(c.device_class)));
        pairs.push(("interface_classes", Json::Arr(c.interface_classes.iter().map(|&k| num(k)).collect())));
    }
    obj(pairs)
}

fn usb_candidate(a: &Accessory) -> Candidate {
    Candidate {
        key: usb_grants::usb_key(a.device.vendor_id, a.device.product_id, &a.serial_number),
        name: a.name.clone(),
        vendor_id: a.device.vendor_id,
        product_id: a.device.product_id,
        device_class: a.device.class,
        interface_classes: a.configuration.as_ref().map(|c| c.interfaces.iter().map(|i| i.class).collect()).unwrap_or_default(),
        hid: false,
        usages: Vec::new(),
    }
}

/// The raw HID devices an app may be offered: every hidraw device but the protected ones (keyboards, keypads, keys).
fn hid_candidate(info: &HidrawInfo) -> Option<Candidate> {
    if info.protected() {
        return None;
    }
    Some(Candidate {
        key: usb_grants::hid_key(info.vendor_id, info.product_id, &info.unique),
        name: info.product.clone(),
        vendor_id: info.vendor_id,
        product_id: info.product_id,
        device_class: 0,
        interface_classes: Vec::new(),
        hid: true,
        usages: info.usages.clone(),
    })
}

fn transfer_name(t: TransferType) -> &'static str {
    match t {
        TransferType::Control => "control",
        TransferType::Isochronous => "isochronous",
        TransferType::Bulk => "bulk",
        TransferType::Interrupt => "interrupt",
    }
}

/// The descriptors of an opened device as an app reads them. `claimable` lists the interfaces the session lets it claim.
fn descriptors_json(device: &DeviceDescriptor, config: &ConfigurationDescriptor, claimable: &[u8]) -> Json {
    let interfaces = config
        .interfaces
        .iter()
        .map(|i| {
            obj(vec![
                ("number", num(i.number)),
                ("alternate", num(i.alternate)),
                ("class", num(i.class)),
                ("subclass", num(i.subclass)),
                ("protocol", num(i.protocol)),
                ("claimable", Json::Bool(claimable.contains(&i.number))),
                (
                    "endpoints",
                    Json::Arr(
                        i.endpoints
                            .iter()
                            .map(|e| {
                                obj(vec![
                                    ("address", num(e.address)),
                                    ("type", text(transfer_name(e.transfer_type()))),
                                    ("direction", text(if e.is_in() { "in" } else { "out" })),
                                    ("max_packet_size", num(e.max_packet_size)),
                                    ("interval", num(e.interval)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    obj(vec![
        ("usb_version", num(device.usb_version)),
        ("device_class", num(device.class)),
        ("device_subclass", num(device.subclass)),
        ("device_protocol", num(device.protocol)),
        ("device_version", num(device.device_version)),
        ("configuration", obj(vec![("value", num(config.value)), ("max_power_ma", num(config.max_power_ma)), ("interfaces", Json::Arr(interfaces))])),
    ])
}

/// One request for a worker.
enum Job {
    Claim(u8),
    Release(u8),
    Alternate(u8, u8),
    Read { endpoint: u8, length: usize, timeout: u32 },
    Write { endpoint: u8, data: Vec<u8>, timeout: u32 },
    Control { setup: SetupPacket, data: Vec<u8>, timeout: u32 },
    ClearHalt(u8),
    Reset,
    HidWrite(Vec<u8>),
    HidGetFeature { report_id: u8, length: usize },
    HidSetFeature(Vec<u8>),
}

impl Job {
    /// The name the `done` event gives the request.
    fn op(&self) -> &'static str {
        match self {
            Job::Claim(_) => "claim",
            Job::Release(_) => "release",
            Job::Alternate(..) => "alternate",
            Job::Read { .. } => "read",
            Job::Write { .. } => "write",
            Job::Control { .. } => "control",
            Job::ClearHalt(_) => "clear_halt",
            Job::Reset => "reset",
            Job::HidWrite(_) => "hid_write",
            Job::HidGetFeature { .. } => "hid_get_feature",
            Job::HidSetFeature(_) => "hid_set_feature",
        }
    }
}

/// The class of a failed request, for an app that reacts to it: `denied` (the rules refused it), `timeout`, `stall`,
/// `disconnected`, `busy`, `overflow` or `other`.
fn failure_of(e: &std::io::Error) -> &'static str {
    use usb_host::Failure;
    match usb_host::classify(e) {
        Failure::Timeout => "timeout",
        Failure::Stall => "stall",
        Failure::Disconnected => "disconnected",
        Failure::Busy => "busy",
        Failure::Overflow => "overflow",
        Failure::Other => "other",
    }
}

/// A failed request: its message and its class.
type Failed = (String, &'static str);

/// What a finished job gives back.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Nothing,
    Data(Vec<u8>),
    Count(usize),
}

/// A message from a worker to the tick.
enum Msg {
    Done { handle: u32, token: u64, op: &'static str, result: Result<Reply, Failed> },
    Report { handle: u32, data: Vec<u8> },
    Failed { handle: u32, error: String },
}

/// Runs one USB job on a session.
fn run_usb_job(session: &mut Session<UsbHostDevice>, job: Job) -> Result<Reply, Failed> {
    let r = match job {
        Job::Claim(n) => session.claim(n).map(|_| Reply::Nothing),
        Job::Release(n) => session.release(n).map(|_| Reply::Nothing),
        Job::Alternate(n, a) => session.select_alternate(n, a).map(|_| Reply::Nothing),
        Job::Read { endpoint, length, timeout } => session.read(endpoint, length, timeout).map(Reply::Data),
        Job::Write { endpoint, data, timeout } => session.write(endpoint, &data, timeout).map(Reply::Count),
        Job::Control { setup, data, timeout } => session.control(&setup, &data, timeout).map(Reply::Data),
        Job::ClearHalt(e) => session.clear_halt(e).map(|_| Reply::Nothing),
        Job::Reset => session.reset().map(|_| Reply::Nothing),
        Job::HidWrite(_) | Job::HidGetFeature { .. } | Job::HidSetFeature(_) => return Err(("not a USB request".into(), "denied")),
    };
    r.map_err(|e| {
        let kind = match &e {
            bifrost_core::usb_session::Error::Denied(_) => "denied",
            bifrost_core::usb_session::Error::Io(io) => failure_of(io),
        };
        (e.to_string(), kind)
    })
}

/// Runs one HID job on a node.
fn run_hid_job(dev: &mut HidrawDevice, job: Job) -> Result<Reply, Failed> {
    let r = match job {
        Job::HidWrite(data) => dev.write_report(&data).map(Reply::Count),
        Job::HidGetFeature { report_id, length } => dev.get_feature(report_id, length).map(Reply::Data),
        Job::HidSetFeature(data) => dev.set_feature(&data).map(Reply::Count),
        _ => return Err(("not a HID request".into(), "denied")),
    };
    r.map_err(|e| (e.to_string(), failure_of(&e)))
}

/// The worker of one open USB device: runs its jobs in order until the handle closes.
fn usb_worker(mut session: Session<UsbHostDevice>, handle: u32, jobs: Receiver<(u64, Job)>, out: Sender<Msg>, inflight: Arc<AtomicUsize>) {
    while let Ok((token, job)) = jobs.recv() {
        let op = job.op();
        let result = run_usb_job(&mut session, job);
        inflight.fetch_sub(1, Ordering::Relaxed);
        if out.send(Msg::Done { handle, token, op, result }).is_err() {
            break;
        }
    }
}

/// The worker of one open HID device: runs its jobs, and between them reads whatever input reports wait.
fn hid_worker(mut dev: HidrawDevice, handle: u32, jobs: Receiver<(u64, Job)>, out: Sender<Msg>, inflight: Arc<AtomicUsize>, queued: Arc<AtomicUsize>) {
    let mut buf = vec![0u8; hidraw::MAX_REPORT];
    loop {
        match jobs.recv_timeout(Duration::from_millis(5)) {
            Ok((token, job)) => {
                let op = job.op();
                let result = run_hid_job(&mut dev, job);
                inflight.fetch_sub(1, Ordering::Relaxed);
                if out.send(Msg::Done { handle, token, op, result }).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        for _ in 0..32 {
            match dev.read_report(&mut buf) {
                Ok(Some(n)) if n > 0 => {
                    if queued.load(Ordering::Relaxed) >= MAX_QUEUED_REPORTS {
                        continue;
                    }
                    queued.fetch_add(1, Ordering::Relaxed);
                    if out.send(Msg::Report { handle, data: buf[..n].to_vec() }).is_err() {
                        return;
                    }
                }
                Ok(_) => break,
                Err(e) => {
                    let _ = out.send(Msg::Failed { handle, error: e.to_string() });
                    return;
                }
            }
        }
    }
}

/// One open device.
struct Open {
    app: String,
    key: String,
    hid: bool,
    jobs: Sender<(u64, Job)>,
    inflight: Arc<AtomicUsize>,
    queued: Arc<AtomicUsize>,
}

pub struct UsbAccessService {
    store: PathBuf,
    sysfs: PathBuf,
    hid_sysfs: PathBuf,
    dev: PathBuf,
    /// The USB bus as the last scan saw it.
    manager: AccessoryManager,
    /// The hidraw devices as the last scan saw them, and the names that scan listed.
    hid_infos: Vec<HidrawInfo>,
    hid_names: Vec<String>,
    last_scan: Option<f64>,
    loaded: bool,
    open: BTreeMap<u32, Open>,
    next_handle: u32,
    next_token: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// The devices that were connected at the last scan, to find plugs and unplugs.
    known: BTreeSet<String>,
}

impl Default for UsbAccessService {
    fn default() -> Self {
        let store = super::super::apphost::root().join(STORE_FILE);
        // unit tests never read the machine's buses
        if cfg!(test) {
            return Self::with_roots(store, Path::new(""), Path::new(""), Path::new(""));
        }
        Self::with_roots(store, Path::new(usb_bus_source::SYSFS_USB_DEVICES), Path::new(hidraw::SYSFS_HIDRAW), Path::new("/dev"))
    }
}

impl UsbAccessService {
    /// A service that reads USB devices from `sysfs`, hidraw devices from `hid_sysfs`, opens nodes under `dev` and keeps
    /// the Always Allow grants in `store`. An empty `sysfs` or `hid_sysfs` turns that bus off.
    pub fn with_roots(store: PathBuf, sysfs: &Path, hid_sysfs: &Path, dev: &Path) -> Self {
        let (tx, rx) = mpsc::channel();
        UsbAccessService {
            store,
            sysfs: sysfs.to_path_buf(),
            hid_sysfs: hid_sysfs.to_path_buf(),
            dev: dev.to_path_buf(),
            manager: AccessoryManager::new(),
            hid_infos: Vec::new(),
            hid_names: Vec::new(),
            last_scan: None,
            loaded: false,
            open: BTreeMap::new(),
            next_handle: 1,
            next_token: 1,
            tx,
            rx,
            known: BTreeSet::new(),
        }
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Ok(t) = std::fs::read_to_string(&self.store) {
            GRANTS.with(|g| {
                let mut g = g.borrow_mut();
                // grants made before the file was read (none in the shell) are kept
                let loaded = Grants::from_text(&t);
                for app in loaded.apps() {
                    for key in loaded.granted_keys(&app) {
                        g.grant(&app, &key, Scope::Always);
                    }
                }
            });
        }
    }

    fn save(&self) {
        if !DIRTY.with(|d| d.replace(false)) {
            return;
        }
        let text = GRANTS.with(|g| g.borrow().to_text());
        if bifrost_core::fsutil::write_atomic(&self.store, text).is_err() {
            DIRTY.with(|d| d.set(true));
        }
    }

    /// Reads both buses and refreshes the live list. Returns the keys that appeared and the keys that went away.
    fn scan(&mut self) -> (Vec<String>, Vec<String>) {
        if !self.sysfs.as_os_str().is_empty() {
            usb_bus_source::poll(&mut self.manager, &self.sysfs);
        }
        if !self.hid_sysfs.as_os_str().is_empty() {
            let mut names: Vec<String> = std::fs::read_dir(&self.hid_sysfs)
                .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default();
            names.sort();
            if names != self.hid_names {
                self.hid_infos = hidraw::list(&self.hid_sysfs, &self.dev);
                self.hid_names = names;
            }
        }
        let mut live: Vec<Candidate> = self.manager.connected_accessories().into_iter().map(usb_candidate).collect();
        live.extend(self.hid_infos.iter().filter_map(hid_candidate));
        let now: BTreeSet<String> = live.iter().map(|c| c.key.clone()).collect();
        let appeared: Vec<String> = now.difference(&self.known).cloned().collect();
        let gone: Vec<String> = self.known.difference(&now).cloned().collect();
        self.known = now;
        LIVE.with(|l| *l.borrow_mut() = live);
        (appeared, gone)
    }

    fn live(hid: bool) -> Vec<Candidate> {
        LIVE.with(|l| l.borrow().iter().filter(|c| c.hid == hid).cloned().collect())
    }

    fn granted(app: &str, key: &str) -> bool {
        GRANTS.with(|g| g.borrow().is_granted(app, key))
    }

    fn close_handle(&mut self, handle: u32) -> Option<Open> {
        self.open.remove(&handle)
    }

    /// Closes every handle that matches `pred`; returns each with its handle number.
    fn close_where(&mut self, pred: impl Fn(&Open) -> bool) -> Vec<(u32, Open)> {
        let handles: Vec<u32> = self.open.iter().filter(|(_, o)| pred(o)).map(|(h, _)| *h).collect();
        handles.into_iter().filter_map(|h| self.close_handle(h).map(|o| (h, o))).collect()
    }

    fn owned(&self, app: &str, params: &Json) -> Result<(u32, &Open), String> {
        let handle = required(params, "handle", u64::from(u32::MAX))? as u32;
        match self.open.get(&handle) {
            Some(o) if o.app == app => Ok((handle, o)),
            _ => Err(format!("no open device {handle}")),
        }
    }

    fn submit(&mut self, app: &str, params: &Json, hid: bool, job: Job) -> Result<Json, String> {
        let token = self.next_token;
        let (_, open) = self.owned(app, params)?;
        if open.hid != hid {
            return Err(if hid { "this handle is a USB device".into() } else { "this handle is a HID device".into() });
        }
        if open.inflight.load(Ordering::Relaxed) >= MAX_INFLIGHT {
            return Err(format!("more than {MAX_INFLIGHT} requests are waiting"));
        }
        open.inflight.fetch_add(1, Ordering::Relaxed);
        if open.jobs.send((token, job)).is_err() {
            open.inflight.fetch_sub(1, Ordering::Relaxed);
            return Err("the device is closed".into());
        }
        self.next_token += 1;
        Ok(obj(vec![("token", num(token as f64))]))
    }

    /// The method handler without the shell: `app_name` names the app on the sheet, `front` says it is in front.
    pub fn handle(&mut self, app: &str, app_name: &str, front: bool, method: &str, params: &Json) -> Result<Json, String> {
        self.ensure_loaded();
        match method {
            "devices" => {
                let hid = is_hid(params)?;
                let list: Vec<Json> = Self::live(hid).iter().filter(|c| Self::granted(app, &c.key)).map(candidate_json).collect();
                Ok(Json::Arr(list))
            }
            "request" => {
                if !front {
                    return Err("only the app in front may ask for a device".into());
                }
                let hid = is_hid(params)?;
                let filters = parse_filters(params.get("filters"))?;
                let live = Self::live(hid);
                let offered = candidates_for(&filters, &live, &|k| Self::granted(app, k));
                let count = offered.len();
                // one sheet per request; the others wait for the next one, and a device is asked about once per run
                let next = offered.into_iter().find(|c| ASKED.with(|a| a.borrow_mut().insert((app.to_string(), c.key.clone()))));
                let asked = next.is_some();
                if let Some(c) = next {
                    privacy_permissions::queue_prompt(app, app_name, Kind::Usb, &c.key);
                }
                Ok(obj(vec![("offered", num(count as f64)), ("asked", Json::Bool(asked)), ("limit", num(MAX_CANDIDATES as f64))]))
            }
            "revoke" => {
                let id = params.get("id").and_then(Json::as_str).ok_or("revoke needs an id")?.to_string();
                let had = GRANTS.with(|g| g.borrow_mut().revoke(app, &id));
                if had {
                    DIRTY.with(|d| d.set(true));
                }
                ASKED.with(|a| a.borrow_mut().remove(&(app.to_string(), id.clone())));
                self.close_where(|o| o.app == app && o.key == id);
                Ok(Json::Bool(had))
            }
            "open" => self.open_device(app, params),
            "close" => {
                let (handle, _) = self.owned(app, params)?;
                self.close_handle(handle);
                Ok(Json::Null)
            }
            "claim" => {
                let n = required(params, "interface", 0xff)? as u8;
                self.submit(app, params, false, Job::Claim(n))
            }
            "release" => {
                let n = required(params, "interface", 0xff)? as u8;
                self.submit(app, params, false, Job::Release(n))
            }
            "alternate" => {
                let n = required(params, "interface", 0xff)? as u8;
                let a = required(params, "alternate", 0xff)? as u8;
                self.submit(app, params, false, Job::Alternate(n, a))
            }
            "read" => {
                let endpoint = required(params, "endpoint", 0xff)? as u8;
                let length = required(params, "length", MAX_BROKER_BYTES as u64)? as usize;
                let timeout = bounded(params, "timeout", u64::from(u32::MAX))?.unwrap_or(0) as u32;
                self.submit(app, params, false, Job::Read { endpoint, length, timeout })
            }
            "write" => {
                let endpoint = required(params, "endpoint", 0xff)? as u8;
                let data = data_param(params)?;
                let timeout = bounded(params, "timeout", u64::from(u32::MAX))?.unwrap_or(0) as u32;
                self.submit(app, params, false, Job::Write { endpoint, data, timeout })
            }
            "control" => {
                let setup = parse_setup(params)?;
                let data = data_param(params)?;
                let timeout = bounded(params, "timeout", u64::from(u32::MAX))?.unwrap_or(0) as u32;
                self.submit(app, params, false, Job::Control { setup, data, timeout })
            }
            "clear_halt" => {
                let endpoint = required(params, "endpoint", 0xff)? as u8;
                self.submit(app, params, false, Job::ClearHalt(endpoint))
            }
            "reset" => self.submit(app, params, false, Job::Reset),
            "hid_write" => {
                let data = data_param(params)?;
                self.submit(app, params, true, Job::HidWrite(data))
            }
            "hid_get_feature" => {
                let report_id = bounded(params, "report_id", 0xff)?.unwrap_or(0) as u8;
                let length = required(params, "length", MAX_BROKER_BYTES as u64)? as usize;
                self.submit(app, params, true, Job::HidGetFeature { report_id, length })
            }
            "hid_set_feature" => {
                let data = data_param(params)?;
                self.submit(app, params, true, Job::HidSetFeature(data))
            }
            other => Err(format!("usb has no method {other}")),
        }
    }

    fn open_device(&mut self, app: &str, params: &Json) -> Result<Json, String> {
        let id = params.get("id").and_then(Json::as_str).ok_or("open needs an id")?.to_string();
        if !Self::granted(app, &id) {
            return Err("this device is not granted to the app; ask with request".into());
        }
        if self.open.values().filter(|o| o.app == app).count() >= MAX_OPEN_PER_APP {
            return Err(format!("at most {MAX_OPEN_PER_APP} devices can be open"));
        }
        if self.open.values().any(|o| o.key == id) {
            return Err("the device is already open".into());
        }
        let handle = self.next_handle;
        let (jobs, job_rx) = mpsc::channel();
        let inflight = Arc::new(AtomicUsize::new(0));
        let queued = Arc::new(AtomicUsize::new(0));
        let hid = id.starts_with("hid:");
        let reply = if hid {
            let info = self
                .hid_infos
                .iter()
                .find(|i| hid_candidate(i).is_some_and(|c| c.key == id))
                .cloned()
                .ok_or("the device is not connected")?;
            if info.protected() {
                return Err("keyboards and security keys cannot be opened".into());
            }
            let device = HidrawDevice::open(&info.node).map_err(|e| format!("{}: {e}", info.node.display()))?;
            let (out, inf, q) = (self.tx.clone(), inflight.clone(), queued.clone());
            std::thread::spawn(move || hid_worker(device, handle, job_rx, out, inf, q));
            obj(vec![
                ("handle", num(handle)),
                ("descriptor", text(hex_encode(&info.descriptor))),
                ("usages", Json::Arr(info.usages.iter().map(|&(p, u)| Json::Arr(vec![num(p), num(u)])).collect())),
                ("bus", text(info.bus.name())),
            ])
        } else {
            let accessory = self
                .manager
                .connected_accessories()
                .into_iter()
                .find(|a| usb_candidate(a).key == id)
                .ok_or("the device is not connected")?;
            let node = usb_host::node_path(&self.dev, (accessory.registry_id >> 16) as u32, (accessory.registry_id & 0xffff) as u32);
            let mut device = UsbHostDevice::open(&node).map_err(|e| format!("{}: {e}", node.display()))?;
            let (descriptor, configs) = device.descriptors().map_err(|e| e.to_string())?;
            let config = configs.into_iter().next().ok_or("the device has no configuration")?;
            let session = Session::new(device, descriptor, config.clone());
            let json = descriptors_json(&descriptor, &config, &session.claimable());
            let (out, inf) = (self.tx.clone(), inflight.clone());
            std::thread::spawn(move || usb_worker(session, handle, job_rx, out, inf));
            let mut pairs = match json {
                Json::Obj(p) => p,
                _ => Vec::new(),
            };
            pairs.insert(0, ("handle".to_string(), num(handle)));
            Json::Obj(pairs)
        };
        self.next_handle += 1;
        self.open.insert(handle, Open { app: app.to_string(), key: id, hid, jobs, inflight, queued });
        Ok(reply)
    }

    /// The events of one tick, after a scan and the grant bookkeeping.
    fn events(&mut self, running: &dyn Fn(&str) -> bool, scanned: Option<(Vec<String>, Vec<String>)>) -> Vec<Event> {
        let mut out = Vec::new();
        // answers of the sheet
        let answers: Vec<(String, String, bool)> = ANSWERS.with(|a| std::mem::take(&mut *a.borrow_mut()));
        for (app, key, granted) in answers {
            let device = LIVE.with(|l| l.borrow().iter().find(|c| c.key == key).map(candidate_json));
            let mut pairs = vec![("id", text(key)), ("granted", Json::Bool(granted))];
            if let (true, Some(device)) = (granted, device) {
                pairs.push(("device", device));
            }
            out.push(Event { app: Some(app), event: "access".into(), body: obj(pairs) });
        }
        if let Some((appeared, gone)) = scanned {
            let apps = GRANTS.with(|g| g.borrow().apps());
            // events go out before Allow Once grants of an unplugged device are dropped
            for key in &appeared {
                let device = LIVE.with(|l| l.borrow().iter().find(|c| &c.key == key).map(candidate_json));
                for app in &apps {
                    if let (true, Some(device)) = (Self::granted(app, key), device.as_ref()) {
                        out.push(Event { app: Some(app.clone()), event: "connected".into(), body: obj(vec![("device", device.clone())]) });
                    }
                }
            }
            for key in &gone {
                for app in &apps {
                    if Self::granted(app, key) {
                        out.push(Event { app: Some(app.clone()), event: "disconnected".into(), body: obj(vec![("id", text(key.clone()))]) });
                    }
                }
                GRANTS.with(|g| g.borrow_mut().device_gone(key));
                ASKED.with(|a| a.borrow_mut().retain(|(_, k)| k != key));
                for (handle, o) in self.close_where(|o| &o.key == key) {
                    out.push(Event { app: Some(o.app), event: "closed".into(), body: obj(vec![("handle", num(handle)), ("reason", text("the device was unplugged"))]) });
                }
            }
        }
        // a handle whose grant was taken back (Settings, or `revoke`) closes
        for (handle, o) in self.close_where(|o| !Self::granted(&o.app, &o.key)) {
            out.push(Event { app: Some(o.app), event: "closed".into(), body: obj(vec![("handle", num(handle)), ("reason", text("access was taken back"))]) });
        }
        // an app that quit loses its Allow Once grants, its open devices and its asked-about list
        let apps: BTreeSet<String> = GRANTS
            .with(|g| g.borrow().apps())
            .into_iter()
            .chain(self.open.values().map(|o| o.app.clone()))
            .chain(ASKED.with(|a| a.borrow().iter().map(|(app, _)| app.clone()).collect::<Vec<_>>()))
            .collect();
        for app in apps.into_iter().filter(|a| !running(a)) {
            GRANTS.with(|g| g.borrow_mut().end_session(&app));
            ASKED.with(|a| a.borrow_mut().retain(|(x, _)| x != &app));
            self.close_where(|o| o.app == app);
        }
        // what the workers finished
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Done { handle, token, op, result } => {
                    let Some(app) = self.open.get(&handle).map(|o| o.app.clone()) else { continue };
                    let mut pairs = vec![("handle", num(handle)), ("token", num(token as f64)), ("op", text(op))];
                    match result {
                        Ok(reply) => {
                            pairs.push(("ok", Json::Bool(true)));
                            match reply {
                                Reply::Nothing => {}
                                Reply::Data(d) => pairs.push(("data", text(hex_encode(&d)))),
                                Reply::Count(n) => pairs.push(("count", num(n as f64))),
                            }
                        }
                        Err((message, failure)) => {
                            pairs.push(("ok", Json::Bool(false)));
                            pairs.push(("error", text(message)));
                            pairs.push(("failure", text(failure)));
                        }
                    }
                    out.push(Event { app: Some(app), event: "done".into(), body: obj(pairs) });
                }
                Msg::Report { handle, data } => {
                    let Some(open) = self.open.get(&handle) else { continue };
                    open.queued.fetch_sub(1, Ordering::Relaxed);
                    out.push(Event { app: Some(open.app.clone()), event: "report".into(), body: obj(vec![("handle", num(handle)), ("data", text(hex_encode(&data)))]) });
                }
                Msg::Failed { handle, error } => {
                    if let Some(o) = self.close_handle(handle) {
                        out.push(Event { app: Some(o.app), event: "closed".into(), body: obj(vec![("handle", num(handle)), ("reason", text(error))]) });
                    }
                }
            }
        }
        out
    }
}

impl SystemService for UsbAccessService {
    fn name(&self) -> &'static str {
        "usb"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        // the manifest permission for the kind of device the call is about; a handle's kind follows its method
        let holds = |permission: &str| super::app_has_permission(app_id, permission);
        let id_kind = |p: &Json| if p.get("id").and_then(Json::as_str).is_some_and(|id| id.starts_with("hid:")) { "hid" } else { "usb" };
        let (permission, allowed) = match method {
            "hid_write" | "hid_get_feature" | "hid_set_feature" => ("hid", holds("hid")),
            "close" => ("usb or hid", holds("usb") || holds("hid")),
            "devices" | "request" => {
                let kind = if is_hid(params)? { "hid" } else { "usb" };
                (kind, holds(kind))
            }
            "open" | "revoke" => {
                let kind = id_kind(params);
                (kind, holds(kind))
            }
            _ => ("usb", holds("usb")),
        };
        if !allowed {
            return Err(format!("the {permission} permission is not in the app's manifest"));
        }
        let name = super::super::apphost::installed().iter().find(|a| a.manifest.id == app_id).map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
        let front = front_app(shell).as_deref() == Some(app_id);
        self.handle(app_id, &name, front, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        self.ensure_loaded();
        let scanned = if self.last_scan.map_or(true, |last| t - last >= SCAN_EVERY) {
            self.last_scan = Some(t);
            Some(self.scan())
        } else {
            None
        };
        let running = |app: &str| shell.ext.iter().flatten().any(|h| h.id == app && h.running());
        let events = self.events(&running, scanned);
        self.save();
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: u64) -> Json {
        Json::Num(v as f64)
    }

    fn obj_of(pairs: Vec<(&str, Json)>) -> Json {
        obj(pairs)
    }

    fn put(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), format!("{text}\n")).unwrap();
    }

    /// A USB device on `root/<port>` with one vendor-specific interface.
    fn plug_usb(root: &Path, port: &str, vid: &str, pid: &str, dev: u32, class: &str, product: &str, serial: &str) {
        let d = root.join(port);
        std::fs::create_dir_all(d.join(format!("{port}:1.0"))).unwrap();
        for (k, v) in [
            ("idVendor", vid),
            ("idProduct", pid),
            ("busnum", "1"),
            ("version", " 2.00"),
            ("bDeviceClass", "00"),
            ("bMaxPacketSize0", "64"),
            ("bNumConfigurations", "1"),
            ("bcdDevice", "0100"),
            ("bmAttributes", "80"),
            ("bMaxPower", "100mA"),
            ("manufacturer", "Acme"),
            ("product", product),
            ("serial", serial),
        ] {
            put(&d, k, v);
        }
        put(&d, "devnum", &dev.to_string());
        let i = d.join(format!("{port}:1.0"));
        put(&i, "bInterfaceNumber", "00");
        put(&i, "bInterfaceClass", class);
        put(&i, "bInterfaceSubClass", "00");
        put(&i, "bInterfaceProtocol", "00");
    }

    fn plug_hid(root: &Path, name: &str, uevent: &str, descriptor: &[u8]) {
        let device = root.join(name).join("device");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::write(device.join("uevent"), uevent).unwrap();
        std::fs::write(device.join("report_descriptor"), descriptor).unwrap();
    }

    const KEYBOARD: &[u8] = &[0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0xc0];
    const VENDOR_PAD: &[u8] = &[0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0xc0];

    struct Rig {
        usb: PathBuf,
        hid: PathBuf,
        svc: UsbAccessService,
    }

    fn rig(name: &str) -> Rig {
        let base = std::env::temp_dir().join(format!("usb-access-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (usb, hid) = (base.join("usb"), base.join("hid"));
        std::fs::create_dir_all(&usb).unwrap();
        std::fs::create_dir_all(&hid).unwrap();
        // the stores are per shell thread: each test runs on its own thread, so they start empty
        let svc = UsbAccessService::with_roots(base.join("grants.txt"), &usb, &hid, &base.join("dev"));
        Rig { usb, hid, svc }
    }

    impl Rig {
        fn scan(&mut self) -> (Vec<String>, Vec<String>) {
            self.svc.ensure_loaded();
            self.svc.scan()
        }

        fn call(&mut self, app: &str, front: bool, method: &str, params: Json) -> Result<Json, String> {
            self.svc.handle(app, "Scope", front, method, &params)
        }
    }

    #[test]
    fn filters_and_setup_packets_are_parsed_with_their_limits() {
        let f = parse_filters(Some(&Json::Arr(vec![obj_of(vec![("vendor_id", n(0x1234)), ("class", n(0xff))]), obj_of(vec![("usage_page", n(0xff00))])]))).unwrap();
        assert_eq!(f[0], Filter { vendor_id: Some(0x1234), class: Some(0xff), ..Default::default() });
        assert_eq!(f[1].usage_page, Some(0xff00));
        assert!(parse_filters(None).unwrap().is_empty());
        assert!(parse_filters(Some(&Json::Null)).unwrap().is_empty());
        assert!(parse_filters(Some(&Json::Arr(vec![obj_of(vec![("vendor_id", n(0x10000))])]))).is_err());
        assert!(parse_filters(Some(&Json::Arr(vec![obj_of(vec![("class", n(256))])]))).is_err());
        assert!(parse_filters(Some(&Json::Arr(vec![Json::Num(1.0)]))).is_err());
        assert!(parse_filters(Some(&Json::Str("x".into()))).is_err());
        assert!(parse_filters(Some(&Json::Arr((0..9).map(|_| obj_of(vec![])).collect()))).is_err());
        assert!(parse_filters(Some(&Json::Arr(vec![obj_of(vec![("vendor_id", Json::Num(-1.0))])]))).is_err());

        let setup = parse_setup(&obj_of(vec![("direction", text("in")), ("kind", text("vendor")), ("recipient", text("interface")), ("request", n(5)), ("value", n(2)), ("index", n(1)), ("length", n(64))])).unwrap();
        assert_eq!((setup.direction, setup.kind, setup.recipient, setup.request, setup.value, setup.index, setup.length), (Direction::DeviceToHost, RequestKind::Vendor, Recipient::Interface, 5, 2, 1, 64));
        assert!(parse_setup(&obj_of(vec![("direction", text("sideways")), ("kind", text("vendor")), ("request", n(1))])).is_err());
        assert!(parse_setup(&obj_of(vec![("direction", text("in")), ("kind", text("vendor"))])).is_err(), "the request number is required");
        assert!(parse_setup(&obj_of(vec![("direction", text("in")), ("kind", text("vendor")), ("request", n(300))])).is_err());
    }

    #[test]
    fn data_is_hex_and_bounded() {
        assert_eq!(data_param(&obj_of(vec![("data", text("00ff10"))])).unwrap(), vec![0, 255, 16]);
        assert!(data_param(&obj_of(vec![("data", text("abc"))])).is_err(), "odd length");
        assert!(data_param(&obj_of(vec![("data", text("zz"))])).is_err());
        assert!(data_param(&obj_of(vec![("data", text("é"))])).is_err(), "non-ASCII never panics");
        assert!(data_param(&obj_of(vec![("data", n(5))])).is_err());
        assert_eq!(data_param(&obj_of(vec![])).unwrap(), Vec::<u8>::new());
        assert!(data_param(&obj_of(vec![("data", text("00".repeat(MAX_BROKER_BYTES + 1)))])).is_err());
        assert!(data_param(&obj_of(vec![("data", text("00".repeat(MAX_BROKER_BYTES)))])).is_ok());
    }

    #[test]
    fn the_scan_lists_usb_and_hid_devices_but_never_protected_ones() {
        let mut r = rig("scan");
        plug_usb(&r.usb, "1-2", "0403", "6001", 7, "ff", "Logic Probe", "A1");
        plug_usb(&r.usb, "1-3", "046d", "c52b", 8, "03", "Keyboard", "");
        plug_usb(&r.usb, "1-4", "0781", "5581", 9, "08", "Stick", "");
        plug_hid(&r.hid, "hidraw0", "HID_ID=0003:0000046D:0000C52B\nHID_NAME=Keyboard\n", KEYBOARD);
        plug_hid(&r.hid, "hidraw1", "HID_ID=0003:00001234:00000002\nHID_NAME=Pad\nHID_UNIQ=P1\n", VENDOR_PAD);
        let (appeared, gone) = r.scan();
        assert!(gone.is_empty());
        let mut keys = appeared.clone();
        keys.sort();
        assert_eq!(keys, vec!["0403:6001:A1".to_string(), "046d:c52b:-".to_string(), "0781:5581:-".to_string(), "hid:1234:0002:P1".to_string()]);
        // the live list holds all of them; eligibility is applied when a request offers them
        let live = UsbAccessService::live(false);
        assert_eq!(live.iter().filter(|c| c.eligible()).count(), 1, "only the vendor-specific device is offered for raw USB");
        assert_eq!(UsbAccessService::live(true).len(), 1, "the keyboard's hidraw node is not even a candidate");
        // unplugging reports it
        std::fs::remove_dir_all(r.usb.join("1-2")).unwrap();
        let (appeared, gone) = r.scan();
        assert!(appeared.is_empty());
        assert_eq!(gone, vec!["0403:6001:A1".to_string()]);
    }

    #[test]
    fn a_device_is_invisible_to_an_app_until_the_person_grants_it() {
        let mut r = rig("consent");
        plug_usb(&r.usb, "1-2", "0403", "6001", 7, "ff", "Logic Probe", "A1");
        r.scan();
        let none = r.call("is.a.scope", true, "devices", obj_of(vec![])).unwrap();
        assert_eq!(none, Json::Arr(vec![]), "an ungranted device is not listed, so it cannot be fingerprinted");
        // a background app may not ask
        assert!(r.call("is.a.scope", false, "request", obj_of(vec![])).is_err());
        // the app in front asks: a sheet is queued, and asking again in the same run queues nothing new
        let asked = r.call("is.a.scope", true, "request", obj_of(vec![("filters", Json::Arr(vec![obj_of(vec![("vendor_id", n(0x0403))])]))])).unwrap();
        assert_eq!(asked.get("offered").and_then(Json::as_u64), Some(1));
        assert_eq!(asked.get("asked"), Some(&Json::Bool(true)));
        let again = r.call("is.a.scope", true, "request", obj_of(vec![])).unwrap();
        assert_eq!(again.get("offered").and_then(Json::as_u64), Some(1));
        assert_eq!(again.get("asked"), Some(&Json::Bool(false)), "the person was already asked in this run");
        let prompt = privacy_permissions::take_prompt().expect("the sheet is queued");
        assert_eq!((prompt.kind, prompt.purpose.as_str(), prompt.app_id.as_str()), (Kind::Usb, "0403:6001:A1", "is.a.scope"));
        assert_eq!(label_of("0403:6001:A1"), "Logic Probe");
        // Don't Allow grants nothing
        privacy_permissions::decide_grant(&prompt, Grant::Deny);
        assert_eq!(r.call("is.a.scope", true, "devices", obj_of(vec![])).unwrap(), Json::Arr(vec![]));
        // a filter that does not match offers nothing
        let other = r.call("is.b.other", true, "request", obj_of(vec![("filters", Json::Arr(vec![obj_of(vec![("vendor_id", n(0x9999))])]))])).unwrap();
        assert_eq!(other.get("offered").and_then(Json::as_u64), Some(0));
        // Allow Once lists it for this app only
        let sheet = r.call("is.b.other", true, "request", obj_of(vec![])).unwrap();
        assert_eq!(sheet.get("asked"), Some(&Json::Bool(true)));
        let prompt = privacy_permissions::take_prompt().unwrap();
        privacy_permissions::decide_grant(&prompt, Grant::Once);
        let mine = r.call("is.b.other", true, "devices", obj_of(vec![])).unwrap();
        assert_eq!(mine.as_array().unwrap().len(), 1);
        assert_eq!(mine.as_array().unwrap()[0].get("id").and_then(Json::as_str), Some("0403:6001:A1"));
        assert_eq!(r.call("is.a.scope", true, "devices", obj_of(vec![])).unwrap(), Json::Arr(vec![]), "the grant is not shared with another app");
    }

    #[test]
    fn protected_devices_are_never_offered_and_keyboards_never_opened() {
        let mut r = rig("protected");
        plug_usb(&r.usb, "1-3", "046d", "c52b", 8, "03", "Keyboard", "");
        plug_usb(&r.usb, "1-4", "0781", "5581", 9, "08", "Stick", "");
        plug_hid(&r.hid, "hidraw0", "HID_ID=0003:0000046D:0000C52B\nHID_NAME=Keyboard\n", KEYBOARD);
        r.scan();
        let usb = r.call("is.a.scope", true, "request", obj_of(vec![])).unwrap();
        assert_eq!(usb.get("offered").and_then(Json::as_u64), Some(0));
        let hid = r.call("is.a.scope", true, "request", obj_of(vec![("kind", text("hid"))])).unwrap();
        assert_eq!(hid.get("offered").and_then(Json::as_u64), Some(0));
        // even a forged grant cannot open a keyboard: the node is not a candidate, so it is "not connected"
        answer("is.a.scope", "hid:046d:c52b:-", Grant::WhileUsing);
        let err = r.call("is.a.scope", true, "open", obj_of(vec![("id", text("hid:046d:c52b:-"))])).unwrap_err();
        assert!(err.contains("not connected"), "{err}");
    }

    #[test]
    fn a_granted_device_that_is_unplugged_loses_its_once_grant_and_tells_the_app() {
        let mut r = rig("unplug");
        plug_usb(&r.usb, "1-2", "0403", "6001", 7, "ff", "Logic Probe", "A1");
        r.scan();
        answer("is.a.scope", "0403:6001:A1", Grant::Once);
        answer("is.a.scope", "0403:6001:B2", Grant::WhileUsing);
        let running = |_: &str| true;
        let first = r.svc.events(&running, Some((Vec::new(), Vec::new())));
        assert_eq!(first.len(), 2, "one access event per sheet answer");
        assert!(first.iter().all(|e| e.event == "access" && e.app.as_deref() == Some("is.a.scope")));
        assert_eq!(first[0].body.get("granted"), Some(&Json::Bool(true)));
        assert!(first[0].body.get("device").is_some(), "a granted, connected device is described");
        assert!(first[1].body.get("device").is_none(), "an unplugged one is not");
        std::fs::remove_dir_all(r.usb.join("1-2")).unwrap();
        let scanned = r.scan();
        let events = r.svc.events(&running, Some(scanned));
        let gone: Vec<&Event> = events.iter().filter(|e| e.event == "disconnected").collect();
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].body.get("id").and_then(Json::as_str), Some("0403:6001:A1"));
        assert!(!UsbAccessService::granted("is.a.scope", "0403:6001:A1"), "Allow Once ended with the unplug");
        assert!(UsbAccessService::granted("is.a.scope", "0403:6001:B2"), "Always Allow stays");
    }

    #[test]
    fn a_plug_of_a_granted_device_is_announced_to_that_app_only() {
        let mut r = rig("plug");
        answer("is.a.scope", "0403:6001:A1", Grant::WhileUsing);
        let running = |_: &str| true;
        r.svc.events(&running, None);
        plug_usb(&r.usb, "1-2", "0403", "6001", 7, "ff", "Logic Probe", "A1");
        plug_usb(&r.usb, "1-5", "1111", "2222", 9, "ff", "Other", "");
        let scanned = r.scan();
        let events = r.svc.events(&running, Some(scanned));
        let connected: Vec<&Event> = events.iter().filter(|e| e.event == "connected").collect();
        assert_eq!(connected.len(), 1, "the other device is silent");
        assert_eq!(connected[0].app.as_deref(), Some("is.a.scope"));
        assert_eq!(connected[0].body.get("device").and_then(|d| d.get("name")).and_then(Json::as_str), Some("Logic Probe"));
    }

    #[test]
    fn a_quit_app_loses_once_grants_and_asked_marks_but_keeps_always() {
        let mut r = rig("quit");
        answer("is.a.scope", "k:once", Grant::Once);
        answer("is.a.scope", "k:always", Grant::WhileUsing);
        ASKED.with(|a| a.borrow_mut().insert(("is.a.scope".into(), "k:denied".into())));
        let still_running = |_: &str| true;
        r.svc.events(&still_running, None);
        assert!(UsbAccessService::granted("is.a.scope", "k:once"));
        let quit = |_: &str| false;
        r.svc.events(&quit, None);
        assert!(!UsbAccessService::granted("is.a.scope", "k:once"));
        assert!(UsbAccessService::granted("is.a.scope", "k:always"));
        assert!(ASKED.with(|a| a.borrow().is_empty()), "asked again on the next run");
    }

    #[test]
    fn always_allow_is_saved_and_read_back_and_once_is_not() {
        let mut r = rig("persist");
        answer("is.a.scope", "0403:6001:A1", Grant::WhileUsing);
        answer("is.a.scope", "0403:6001:B2", Grant::Once);
        r.svc.ensure_loaded();
        r.svc.save();
        let saved = std::fs::read_to_string(&r.svc.store).unwrap();
        assert_eq!(saved, "is.a.scope 0403:6001:A1\n");
        GRANTS.with(|g| *g.borrow_mut() = Grants::new());
        let mut again = UsbAccessService::with_roots(r.svc.store.clone(), Path::new(""), Path::new(""), Path::new(""));
        again.ensure_loaded();
        assert!(UsbAccessService::granted("is.a.scope", "0403:6001:A1"));
        assert!(!UsbAccessService::granted("is.a.scope", "0403:6001:B2"));
        // revoking from the app removes it from the file
        let out = again.handle("is.a.scope", "Scope", true, "revoke", &obj_of(vec![("id", text("0403:6001:A1"))])).unwrap();
        assert_eq!(out, Json::Bool(true));
        again.save();
        assert_eq!(std::fs::read_to_string(&again.store).unwrap(), "");
    }

    #[test]
    fn open_needs_a_grant_a_connected_device_and_a_handle_belongs_to_its_app() {
        let mut r = rig("open");
        plug_usb(&r.usb, "1-2", "0403", "6001", 7, "ff", "Logic Probe", "A1");
        r.scan();
        let err = r.call("is.a.scope", true, "open", obj_of(vec![("id", text("0403:6001:A1"))])).unwrap_err();
        assert!(err.contains("not granted"), "{err}");
        answer("is.a.scope", "0403:6001:A1", Grant::Once);
        // the usbfs node does not exist in the rig, so the open fails after the checks, with a clear error
        let err = r.call("is.a.scope", true, "open", obj_of(vec![("id", text("0403:6001:A1"))])).unwrap_err();
        assert!(err.contains("007") || err.contains("No such file"), "{err}");
        // a handle that was never opened is refused for every call, and another app's handle is not reachable
        for m in ["close", "claim", "release", "read", "write", "control", "clear_halt", "reset", "hid_write", "hid_get_feature", "hid_set_feature", "alternate"] {
            let p = obj_of(vec![("handle", n(1)), ("interface", n(0)), ("alternate", n(0)), ("endpoint", n(0x81)), ("length", n(4)), ("direction", text("in")), ("kind", text("vendor")), ("request", n(1))]);
            assert!(r.call("is.a.scope", true, m, p).is_err(), "{m}");
        }
        assert!(r.call("is.a.scope", true, "nonsense", obj_of(vec![])).is_err());
        assert!(r.call("is.a.scope", true, "devices", obj_of(vec![("kind", text("bluetooth"))])).is_err());
    }

    #[test]
    fn the_descriptors_reach_the_app_with_the_claimable_interfaces_marked() {
        use bifrost_core::usb_descriptors::{EndpointDescriptor, InterfaceDescriptor};
        let device = DeviceDescriptor { usb_version: 0x0200, class: 0, subclass: 0, protocol: 0, max_packet_size0: 64, vendor_id: 1, product_id: 2, device_version: 0x0100, configurations: 1 };
        let config = ConfigurationDescriptor {
            value: 1,
            attributes: 0x80,
            max_power_ma: 100,
            interfaces: vec![
                InterfaceDescriptor { number: 0, alternate: 0, class: 0xff, subclass: 0, protocol: 0, endpoints: vec![EndpointDescriptor { address: 0x81, attributes: 2, max_packet_size: 64, interval: 0 }] },
                InterfaceDescriptor { number: 1, alternate: 0, class: 0x03, subclass: 0, protocol: 0, endpoints: vec![] },
            ],
        };
        let json = descriptors_json(&device, &config, &[0]);
        let ifaces = json.get("configuration").and_then(|c| c.get("interfaces")).and_then(Json::as_array).unwrap();
        assert_eq!(ifaces[0].get("claimable"), Some(&Json::Bool(true)));
        assert_eq!(ifaces[1].get("claimable"), Some(&Json::Bool(false)));
        let ep = &ifaces[0].get("endpoints").and_then(Json::as_array).unwrap()[0];
        assert_eq!(ep.get("type").and_then(Json::as_str), Some("bulk"));
        assert_eq!(ep.get("direction").and_then(Json::as_str), Some("in"));
        assert_eq!(ep.get("address").and_then(Json::as_u64), Some(0x81));
    }
}
