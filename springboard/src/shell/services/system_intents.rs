//! The `system_intents` service: the shell serves the App Intents that no app owns (`bifrost_app::system_intents`,
//! `vpn_intents`, `scanner_intents`) so Search, Shortcuts and first-party apps can run them.
//!
//! Methods (params are a JSON object):
//! - `list {}` returns `[{id, title}]`, the intents this service runs.
//! - `run {intent, ...}` runs one and returns `{status: "queued"}` at once; the work happens off the call. Only
//!   first-party apps (`is.olibuijr.*`) may call it: it locks the screen, changes the network and starts a scan.
//!   - `lock_screen`: locks the screen on the shell's next tick, as Lock Screen in the menu does.
//!   - `vpn_connect {profile}`: brings the named NetworkManager VPN connection up (`desktop_network::VpnAction`).
//!   - `vpn_disconnect {profile?}`: brings the named VPN connection down, or every VPN connection that is up.
//!   - `scan_document {device?, format?}`: scans one page to `Documents/Scan <time>.<ext>` in the account's home. The
//!     scanner is named by its display name (the first one found when absent). The format is `jpeg` (the default),
//!     `png` or `tiff`; `pdf` is refused because the scan command cannot write one.
//!   - `scan_to_folder {folder, device?}`: the same, into the named folder under the home folder (JPEG).
//! - `last {}` returns `{message, ok}` for the last scan: where the file went, or why it failed. `message` is empty
//!   before any scan ran.
//!
//! Nothing here reports a result that did not happen: a scan or a tunnel that fails leaves its reason in `last` (scans)
//! or in the network service's error (VPN), and the reply to `run` only says the request was accepted.
use super::{Shell, SystemService};
use crate::shell::desktop_network::{self, valid_vpn_name, VpnAction};
use bifrost_core::json::Json;
use bifrost_ui::scan_job::{DocumentFormat, ScanSettings};
use bifrost_ui::scanner;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Intent ids, as the SDK declares them.
pub const LOCK_SCREEN: &str = "lock_screen";
pub const VPN_CONNECT: &str = "vpn_connect";
pub const VPN_DISCONNECT: &str = "vpn_disconnect";
pub const SCAN_DOCUMENT: &str = "scan_document";
pub const SCAN_TO_FOLDER: &str = "scan_to_folder";

/// Every intent this service runs, with the title Search lists.
pub const INTENTS: [(&str, &str); 5] = [
    (LOCK_SCREEN, "Lock Screen"),
    (VPN_CONNECT, "Connect VPN"),
    (VPN_DISCONNECT, "Disconnect VPN"),
    (SCAN_DOCUMENT, "Scan Document"),
    (SCAN_TO_FOLDER, "Scan to Folder"),
];

/// The folder `scan_document` saves into, under the home folder.
pub const DOCUMENTS: &str = "Documents";
/// The longest folder path `scan_to_folder` takes, in bytes.
pub const FOLDER_MAX: usize = 128;

/// One checked request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Lock,
    VpnConnect(String),
    VpnDisconnect(Option<String>),
    Scan(Scan),
}

/// A scan request: which scanner (by display name, the first one found when `None`), the file format, and the folder
/// under the home folder the file goes in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub device: Option<String>,
    pub format: DocumentFormat,
    pub folder: String,
}

fn string_param<'a>(params: &'a Json, name: &str) -> Result<Option<&'a str>, String> {
    match params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Str(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("{name} must be text")),
    }
}

/// The format a `format` parameter names. `pdf` is refused: `scanimage` writes images only.
fn format_of(name: &str) -> Result<DocumentFormat, String> {
    match name {
        "jpeg" => Ok(DocumentFormat::Jpeg),
        "png" => Ok(DocumentFormat::Png),
        "tiff" => Ok(DocumentFormat::Tiff),
        "pdf" => Err("PDF scans are not supported yet".into()),
        other => Err(format!("{other} is not a scan format")),
    }
}

/// Whether `folder` can be a folder under the home folder: relative, made of plain names, not too long.
pub fn valid_folder(folder: &str) -> bool {
    !folder.is_empty()
        && folder.len() <= FOLDER_MAX
        && !folder.starts_with('/')
        && !folder.chars().any(char::is_control)
        && folder.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Checks the intent `id` with `params` and returns what to run.
pub fn parse(id: &str, params: &Json) -> Result<Intent, String> {
    match id {
        LOCK_SCREEN => Ok(Intent::Lock),
        VPN_CONNECT => {
            let name = string_param(params, "profile")?.ok_or("vpn_connect needs a profile")?;
            if !valid_vpn_name(name) {
                return Err("That is not a VPN configuration name".into());
            }
            Ok(Intent::VpnConnect(name.to_string()))
        }
        VPN_DISCONNECT => match string_param(params, "profile")? {
            None => Ok(Intent::VpnDisconnect(None)),
            Some(name) if valid_vpn_name(name) => Ok(Intent::VpnDisconnect(Some(name.to_string()))),
            Some(_) => Err("That is not a VPN configuration name".into()),
        },
        SCAN_DOCUMENT => {
            let format = match string_param(params, "format")? {
                Some(name) => format_of(name)?,
                None => DocumentFormat::Jpeg,
            };
            Ok(Intent::Scan(Scan { device: string_param(params, "device")?.map(str::to_string), format, folder: DOCUMENTS.into() }))
        }
        SCAN_TO_FOLDER => {
            let folder = string_param(params, "folder")?.ok_or("scan_to_folder needs a folder")?;
            if !valid_folder(folder) {
                return Err("The folder must be a plain path under the home folder".into());
            }
            Ok(Intent::Scan(Scan { device: string_param(params, "device")?.map(str::to_string), format: DocumentFormat::Jpeg, folder: folder.to_string() }))
        }
        other => Err(format!("no intent {other}")),
    }
}

/// `YYYY-MM-DD HH.MM.SS` (UTC) of a Unix time, for a scan's file name.
pub fn stamp(unix_secs: u64) -> String {
    let (y, mo, d, h, mi, s) = bifrost_core::snapshot_backup::civil(unix_secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}.{mi:02}.{s:02}")
}

/// The scanner to use: the one whose display name (or device name) is `wanted`, else the first one found.
pub fn pick_scanner(list: &[scanner::ScannerInfo], wanted: Option<&str>) -> Result<String, String> {
    match wanted {
        Some(name) => list
            .iter()
            .find(|s| s.name() == name || s.device == name)
            .map(|s| s.device.clone())
            .ok_or_else(|| format!("No scanner named {name} was found")),
        None => list.first().map(|s| s.device.clone()).ok_or_else(|| "No scanner was found".to_string()),
    }
}

/// Where a scan is saved: `<home>/<folder>/Scan <stamp>.<ext>`.
pub fn scan_path(home: &Path, scan: &Scan, settings: &ScanSettings, unix_secs: u64) -> PathBuf {
    home.join(&scan.folder).join(scanner::scan_file_name(&settings.document_name, &stamp(unix_secs), scan.format))
}

/// Runs a scan to completion on this thread and returns the file it wrote.
fn run_scan(home: &Path, scan: &Scan, unix_secs: u64) -> Result<PathBuf, String> {
    let list = scanner::list().map_err(|e| e.to_string())?;
    let device = pick_scanner(&list, scan.device.as_deref())?;
    let settings = ScanSettings { format: scan.format, ..ScanSettings::default() };
    let path = scan_path(home, scan, &settings, unix_secs);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    scanner::scan_to(&device, &settings, false, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The outcome of the last scan: the message and whether it worked. Empty before any scan.
static LAST: Mutex<(String, bool)> = Mutex::new((String::new(), false));

fn set_last(message: String, ok: bool) {
    if let Ok(mut last) = LAST.lock() {
        *last = (message, ok);
    }
}

/// A Lock Screen request waiting for the shell's tick (a service cannot return effects).
static LOCK: AtomicBool = AtomicBool::new(false);

/// Whether a Lock Screen request came in since the last call. The shell's tick locks the screen when it did.
pub fn take_lock() -> bool {
    LOCK.swap(false, Ordering::AcqRel)
}

fn start_scan(scan: Scan) {
    let home = crate::shell::dock_stack::home_dir();
    let _ = std::thread::Builder::new().name("scan-intent".into()).spawn(move || {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        match run_scan(&home, &scan, now) {
            Ok(path) => set_last(format!("Saved {}", path.display()), true),
            Err(e) => set_last(e, false),
        }
    });
}

/// Carries out a checked request.
fn start(intent: Intent) {
    match intent {
        Intent::Lock => LOCK.store(true, Ordering::Release),
        Intent::VpnConnect(name) => desktop_network::send(desktop_network::Request::Vpn(VpnAction::Up(name))),
        Intent::VpnDisconnect(name) => desktop_network::send(desktop_network::Request::Vpn(VpnAction::Down(name))),
        Intent::Scan(scan) => start_scan(scan),
    }
}

/// Whether `app_id` is a first-party app, the only kind that may run these intents.
pub fn first_party(app_id: &str) -> bool {
    app_id.starts_with("is.olibuijr.")
}

pub struct SystemIntents;

impl SystemService for SystemIntents {
    fn name(&self) -> &'static str {
        "system_intents"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "list" => Ok(Json::Arr(
                INTENTS.iter().map(|(id, title)| Json::Obj(vec![("id".into(), Json::Str((*id).into())), ("title".into(), Json::Str((*title).into()))])).collect(),
            )),
            "run" => {
                if !first_party(app_id) {
                    return Err("system_intents: only first-party apps may run system intents".into());
                }
                let id = string_param(params, "intent")?.ok_or("run needs an intent")?;
                let intent = parse(id, params)?;
                start(intent);
                Ok(Json::Obj(vec![("status".into(), Json::Str("queued".into()))]))
            }
            "last" => {
                let (message, ok) = LAST.lock().map(|l| l.clone()).unwrap_or_default();
                Ok(Json::Obj(vec![("message".into(), Json::Str(message)), ("ok".into(), Json::Bool(ok))]))
            }
            other => Err(format!("system_intents: no method {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: &[(&str, &str)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| ((*k).to_string(), Json::Str((*v).to_string()))).collect())
    }

    #[test]
    fn lock_screen_takes_no_parameters_and_queues_once() {
        assert_eq!(parse(LOCK_SCREEN, &Json::Obj(vec![])), Ok(Intent::Lock));
        let _ = take_lock();
        start(Intent::Lock);
        assert!(take_lock(), "the request waits for the tick");
        assert!(!take_lock(), "and is taken once");
    }

    #[test]
    fn vpn_intents_check_the_name() {
        assert_eq!(parse(VPN_CONNECT, &obj(&[("profile", "Office VPN")])), Ok(Intent::VpnConnect("Office VPN".into())));
        assert!(parse(VPN_CONNECT, &obj(&[])).is_err(), "connect needs a profile");
        assert!(parse(VPN_CONNECT, &obj(&[("profile", "--help")])).is_err());
        assert_eq!(parse(VPN_DISCONNECT, &obj(&[])), Ok(Intent::VpnDisconnect(None)));
        assert_eq!(parse(VPN_DISCONNECT, &obj(&[("profile", "Office VPN")])), Ok(Intent::VpnDisconnect(Some("Office VPN".into()))));
        assert!(parse(VPN_DISCONNECT, &obj(&[("profile", "a\nb")])).is_err());
        assert!(parse(VPN_CONNECT, &Json::Obj(vec![("profile".into(), Json::Num(1.0))])).is_err(), "a number is not a name");
    }

    #[test]
    fn scan_document_defaults_to_jpeg_in_documents_and_refuses_pdf() {
        let scan = |device: Option<&str>, format: DocumentFormat, folder: &str| Intent::Scan(Scan { device: device.map(str::to_string), format, folder: folder.into() });
        assert_eq!(parse(SCAN_DOCUMENT, &obj(&[])), Ok(scan(None, DocumentFormat::Jpeg, "Documents")));
        assert_eq!(parse(SCAN_DOCUMENT, &obj(&[("device", "Brother MFC"), ("format", "png")])), Ok(scan(Some("Brother MFC"), DocumentFormat::Png, "Documents")));
        assert_eq!(parse(SCAN_DOCUMENT, &obj(&[("format", "pdf")])), Err("PDF scans are not supported yet".into()));
        assert!(parse(SCAN_DOCUMENT, &obj(&[("format", "gif")])).is_err());
    }

    #[test]
    fn scan_to_folder_stays_under_the_home_folder() {
        let ok = parse(SCAN_TO_FOLDER, &obj(&[("folder", "Scans/2026")]));
        assert_eq!(ok, Ok(Intent::Scan(Scan { device: None, format: DocumentFormat::Jpeg, folder: "Scans/2026".into() })));
        for bad in ["", "/etc", "../x", "a/../b", "a//b", "./a", "a\nb"] {
            assert!(parse(SCAN_TO_FOLDER, &obj(&[("folder", bad)])).is_err(), "{bad:?}");
        }
        let long = "a".repeat(FOLDER_MAX + 1);
        assert!(parse(SCAN_TO_FOLDER, &obj(&[("folder", long.as_str())])).is_err());
        assert!(parse(SCAN_TO_FOLDER, &obj(&[])).is_err(), "the folder is required");
    }

    #[test]
    fn an_unknown_intent_is_an_error() {
        assert_eq!(parse("reboot", &obj(&[])), Err("no intent reboot".into()));
    }

    #[test]
    fn the_scanner_is_picked_by_display_name_device_or_first() {
        let list = scanner::parse_list("device 'airscan:w0:Brother' is a Brother MFC-L2750DW series\ndevice 'genesys:libusb:001,004' is a Canon LiDE 220 flatbed scanner\n");
        assert_eq!(pick_scanner(&list, None), Ok("airscan:w0:Brother".into()));
        assert_eq!(pick_scanner(&list, Some("Canon LiDE 220 flatbed scanner")), Ok("genesys:libusb:001,004".into()));
        assert_eq!(pick_scanner(&list, Some("genesys:libusb:001,004")), Ok("genesys:libusb:001,004".into()));
        assert_eq!(pick_scanner(&list, Some("Nope")), Err("No scanner named Nope was found".into()));
        assert_eq!(pick_scanner(&[], None), Err("No scanner was found".into()));
    }

    #[test]
    fn a_scan_is_named_by_its_time_and_saved_under_the_folder() {
        assert_eq!(stamp(0), "1970-01-01 00.00.00");
        assert_eq!(stamp(1_792_233_845), "2026-10-17 10.44.05");
        let scan = Scan { device: None, format: DocumentFormat::Png, folder: "Scans".into() };
        let path = scan_path(Path::new("/home/me"), &scan, &ScanSettings::default(), 1_792_233_845);
        assert_eq!(path, PathBuf::from("/home/me/Scans/Scan 2026-10-17 10.44.05.png"));
    }

    #[test]
    fn only_first_party_apps_run_intents_and_every_listed_intent_parses() {
        assert!(first_party("is.olibuijr.shortcuts.desktop"));
        assert!(!first_party("com.example.app"));
        assert!(!first_party("is.olibuijr"), "the prefix needs its dot");
        for (id, _) in INTENTS {
            let params = obj(&[("profile", "Office"), ("folder", "Scans")]);
            assert!(parse(id, &params).is_ok(), "{id}");
        }
    }
}
