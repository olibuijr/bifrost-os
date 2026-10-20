//! Settings > Print & Scan: the printers the local printing service knows (`bifrost_ui::cups`, IPP over the local
//! scheduler), the default printer, and adding a network or USB printer by its device address. The list is read on
//! every build of the page, so a change shows at once. The printing service being down is shown as the page's
//! footer, not as a failure of the whole pane. Edits (the default printer, adding a queue) talk to the scheduler on a
//! worker thread, like the read does; the page shows "Working…" until the answer lands.
use super::forms::{Prompt, Purpose};
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use bifrost_ui::cups::{self, Client, Printer, PrinterState};
use bifrost_ui::scanner::{self, ScannerInfo};

/// What a row on the page does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// makes this queue the default printer
    SetDefault(String),
    /// opens the address prompt for a new printer
    Add,
}

/// The printers as the scheduler has them now, or why they could not be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub printers: Vec<Printer>,
    pub error: Option<String>,
    /// the scanners `scanimage -L` lists (Scanners group)
    pub scanners: Vec<ScannerInfo>,
    /// why the scanners could not be listed (scanning not installed, or the command failed)
    pub scanner_error: Option<String>,
}

impl View {
    /// Reads the printers and the scanners. Always returns a view: a read that panics or runs too long is an error
    /// on the page, never a page left on "Loading…".
    pub fn load() -> View {
        let printers = std::panic::catch_unwind(|| Client::local().printers().map_err(|e| e.to_string()))
            .unwrap_or_else(|_| Err("the printer list could not be read".to_string()));
        let scanners = bounded(SCAN_WAIT + std::time::Duration::from_secs(2), scanners_now);
        let v = View::from_result(printers).with_scanners(scanners);
        // the page says it plainly; the reason goes to the log
        if let Some(e) = &v.error {
            bifrost_core::log::shell_logger("print").notice(format!("printing service unavailable: {e}"));
        }
        if let Some(e) = &v.scanner_error {
            bifrost_core::log::shell_logger("print").notice(format!("scanning unavailable: {e}"));
        }
        v
    }

    pub fn from_result(r: Result<Vec<Printer>, String>) -> View {
        match r {
            Ok(printers) => View { printers, error: None, scanners: Vec::new(), scanner_error: None },
            Err(e) => View { printers: Vec::new(), error: Some(e), scanners: Vec::new(), scanner_error: None },
        }
    }

    pub fn with_scanners(mut self, r: Result<Vec<ScannerInfo>, String>) -> View {
        match r {
            Ok(scanners) => {
                self.scanners = scanners;
                self.scanner_error = None;
            }
            Err(e) => self.scanner_error = Some(e),
        }
        self
    }
}

/// How long the page waits for `scanimage -L` (it can sit on a network scan) before it shows the page without scanners;
/// the command is killed then, so a hung search leaves no process behind.
const SCAN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a scanner list is reused: every page open, default change and add reloads the page, and a network scan
/// search is slow and noisy, so reloads inside this window show the last list.
const SCAN_REUSE: std::time::Duration = std::time::Duration::from_secs(15);

type ScanSlot = std::sync::Mutex<Option<(std::time::Instant, Result<Vec<ScannerInfo>, String>)>>;

static SCANS: ScanSlot = std::sync::Mutex::new(None);

/// The scanners now: the list from the last 15 seconds when there is one, else one `scanimage -L` run. Loads that
/// arrive while a run is going wait for it (the lock is held) and share its answer instead of starting another.
fn scanners_now() -> Result<Vec<ScannerInfo>, String> {
    cached(&SCANS, SCAN_REUSE, || scanner::list_within(SCAN_WAIT).map_err(|e| e.to_string()))
}

/// `read`'s answer, reused for `reuse` after it was taken; concurrent callers share one read.
fn cached<T: Clone>(slot: &std::sync::Mutex<Option<(std::time::Instant, T)>>, reuse: std::time::Duration, read: impl FnOnce() -> T) -> T {
    let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, value)) = slot.as_ref() {
        if at.elapsed() < reuse {
            return value.clone();
        }
    }
    let value = read();
    *slot = Some((std::time::Instant::now(), value.clone()));
    value
}

/// Runs `read` on its own thread and waits at most `wait` for the answer; a read that is slower or panics is an error.
fn bounded<T: Send + 'static>(wait: std::time::Duration, read: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read());
    });
    rx.recv_timeout(wait).unwrap_or_else(|_| Err("the read took too long".to_string()))
}

/// The state a printer's row shows.
pub fn state_label(state: PrinterState) -> &'static str {
    match state {
        PrinterState::Idle => "Ready",
        PrinterState::Processing => "Printing",
        PrinterState::Stopped => "Stopped",
        PrinterState::Other(_) => "Unknown",
    }
}

/// What the page says when the printing service cannot be reached (the reason is in the log, not on the page).
const NOT_RUNNING: &str = "The printing service is not running.";

/// The page's groups: the printers with the default one checked, then the add row.
pub fn groups(v: &View) -> Vec<Group> {
    let mut out = Vec::new();
    match &v.error {
        Some(_) => out.push(Group::new(vec![Item::new("No Printing Service").tint(Tint::Dim)]).footer(NOT_RUNNING).header("PRINTERS")),
        None if v.printers.is_empty() => out.push(Group::new(vec![Item::new("No Printers").tint(Tint::Dim)]).header("PRINTERS").footer("Add a printer to print from any app.")),
        None => {
            let rows = v
                .printers
                .iter()
                .map(|p| Item::new(p.name.as_str()).value(state_label(p.state)).check(p.is_default, Action::PrintScan(Edit::SetDefault(p.name.clone()))))
                .collect();
            out.push(Group::new(rows).header("PRINTERS").footer("The default printer is checked. Apps print there unless you choose another."));
        }
    }
    if v.error.is_none() {
        out.push(Group::new(vec![Item::new("Add Printer…").tint(Tint::Blue).action(Action::PrintScan(Edit::Add))])
            .footer("Adds a network or USB printer by its address, such as ipp://printer.local/ipp/print or socket://192.168.1.20. The printer must support driverless printing."));
    }
    out.push(scanners_group(v));
    out
}

/// The SCANNERS group: each scanner the system lists, with how it is connected. Scanning itself is in the Image
/// Capture app; this group only shows what is attached.
pub fn scanners_group(v: &View) -> Group {
    match &v.scanner_error {
        Some(_) => Group::new(vec![Item::new("No Scanners").tint(Tint::Dim)]).header("SCANNERS").footer("Scanning is not available on this computer."),
        None if v.scanners.is_empty() => Group::new(vec![Item::new("No Scanners").tint(Tint::Dim)])
            .header("SCANNERS")
            .footer("Scanners on the network or over USB appear here when they are on. Scan with the Image Capture app."),
        None => {
            let rows = v.scanners.iter().map(|s| Item::new(s.name()).value(transport_label(s))).collect();
            Group::new(rows).header("SCANNERS").footer("Scan with the Image Capture app.")
        }
    }
}

/// How a scanner is connected, as the row shows it.
fn transport_label(s: &ScannerInfo) -> &'static str {
    match s.transport() {
        bifrost_ui::device_browser::Transport::Usb => "USB",
        bifrost_ui::device_browser::Transport::Network => "Network",
        bifrost_ui::device_browser::Transport::Other => "",
    }
}

/// The whole Print & Scan page; `None` while the printers and scanners are read (a placeholder row).
pub fn spec(v: Option<&View>) -> Spec {
    spec_pending(v, false)
}

/// The page with a "Working…" row on top while an edit is running on its worker.
pub fn spec_pending(v: Option<&View>, pending: bool) -> Spec {
    let mut groups = match v {
        Some(v) => groups(v),
        None => vec![Group::new(vec![Item::new("Printers").value("Loading…").tint(Tint::Dim)])],
    };
    if pending {
        groups.insert(0, Group::new(vec![Item::new("Printer Change").value("Working…").tint(Tint::Dim)]));
    }
    Spec { id: PageId::PrintScan, title: "Print & Scan".into(), large: false, groups }
}

/// What a finished edit on the worker hands back to the page (taken once).
#[derive(Default)]
pub enum Landed {
    /// nothing has finished
    #[default]
    Idle,
    /// the edit ended: the next prompt to open, or why it did not happen
    Edited(Result<Option<Prompt>, String>),
}

/// The edit as the worker runs it: [`apply`] or [`finish`], and a panic becomes an error, never a page left on "Working…".
pub fn run_edit(work: impl FnOnce() -> Result<Option<Prompt>, String>) -> Landed {
    Landed::Edited(std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| Err("the printer change could not be made".to_string())))
}

/// Applies a Print & Scan edit: the default printer changes now (`None`), the address prompt opens (`Some`).
/// The default-printer change talks to the scheduler, so the shell runs this on a worker (see [`run_edit`]).
pub fn apply(edit: &Edit) -> Result<Option<Prompt>, String> {
    match edit {
        Edit::SetDefault(name) => Client::local().set_default(name).map(|_| None).map_err(|e| e.to_string()),
        Edit::Add => Ok(Some(Prompt::printer_address())),
    }
}

/// Finishes the address prompt: adds the queue through the scheduler. Other purposes are not this page's.
pub fn finish(purpose: &Purpose, text: &str) -> Result<(), String> {
    if *purpose != Purpose::PrinterAddress {
        return Ok(());
    }
    cups::add_device(&mut Client::local(), text).map(|_| ()).map_err(|e| match e {
        cups::CupsError::BadDeviceUri(device) => format!("unsupported device address {device:?}"),
        // the scheduler may still create the queue after the reply was lost: the page re-lists, so say where to look
        cups::CupsError::Io(why) => format!("printing service did not answer: {why}. If the printer was added, it shows in the list."),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer(name: &str, state: PrinterState, is_default: bool) -> Printer {
        Printer {
            name: name.into(),
            uri: cups::printer_uri(name),
            state,
            state_message: String::new(),
            accepting_jobs: true,
            is_default,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: String::new(),
            queued_jobs: 0,
            state_reasons: Vec::new(),
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    #[test]
    fn states_have_plain_labels() {
        assert_eq!(state_label(PrinterState::Idle), "Ready");
        assert_eq!(state_label(PrinterState::Processing), "Printing");
        assert_eq!(state_label(PrinterState::Stopped), "Stopped");
        assert_eq!(state_label(PrinterState::Other(9)), "Unknown");
    }

    #[test]
    fn the_default_printer_is_checked_and_each_row_asks_to_make_itself_default() {
        let v = View::from_result(Ok(vec![printer("Office", PrinterState::Idle, true), printer("Lab", PrinterState::Stopped, false)]));
        let g = groups(&v);
        assert_eq!(g[0].items.len(), 2);
        assert_eq!(g[0].items[0].value, "Ready");
        assert_eq!(g[0].items[1].value, "Stopped");
        assert_eq!(g[0].items[1].action, Action::PrintScan(Edit::SetDefault("Lab".into())));
        assert_eq!(g[1].items[0].action, Action::PrintScan(Edit::Add));
    }

    #[test]
    fn no_printing_service_says_so_plainly_and_offers_no_add_row() {
        let v = View::from_result(Err("printing service unreachable: refused".into()));
        let g = groups(&v);
        assert_eq!(g.len(), 2, "the printers group, then the scanners group");
        assert_eq!(g[0].footer.as_deref(), Some("The printing service is not running."), "a plain message, not the error");
        assert_eq!(g[1].header.as_deref(), Some("SCANNERS"));
    }

    #[test]
    fn scanners_are_listed_with_how_they_connect() {
        let v = View::from_result(Ok(vec![])).with_scanners(Ok(vec![
            ScannerInfo { device: "genesys:libusb:001,004".into(), description: "Canon LiDE 220".into() },
            ScannerInfo { device: "airscan:w0:Brother".into(), description: "Brother MFC".into() },
        ]));
        let g = groups(&v);
        let scanners = g.last().unwrap();
        assert_eq!(scanners.items[0].label, "Canon LiDE 220");
        assert_eq!(scanners.items[0].value, "USB");
        assert_eq!(scanners.items[1].value, "Network");
        assert_eq!(scanners.items[0].action, Action::None, "a scanner row has no action here");
    }

    #[test]
    fn a_missing_scanner_command_says_so_in_its_group() {
        let v = View::from_result(Ok(vec![])).with_scanners(Err("Scanning is not available: no such file".into()));
        let scanners = groups(&v).pop().unwrap();
        assert_eq!(scanners.items[0].label, "No Scanners");
        assert_eq!(scanners.footer.as_deref(), Some("Scanning is not available on this computer."), "the command's error stays in the log");
    }

    #[test]
    fn an_empty_list_still_offers_to_add_a_printer() {
        let g = groups(&View::from_result(Ok(vec![])));
        assert_eq!(g[0].items[0].label, "No Printers");
        assert_eq!(g[1].items[0].action, Action::PrintScan(Edit::Add));
    }

    fn labels(spec: &Spec) -> Vec<String> {
        spec.groups.iter().flat_map(|g| g.items.iter().map(|i| format!("{} {}", i.label, i.value))).collect()
    }

    #[test]
    fn an_empty_list_shows_no_printers_and_the_add_row_never_loading() {
        let v = View::from_result(Ok(vec![]));
        let spec = spec(Some(&v));
        let all = labels(&spec);
        assert!(all.iter().any(|l| l.starts_with("No Printers")), "{all:?}");
        assert!(all.iter().any(|l| l.starts_with("Add Printer")), "{all:?}");
        assert!(!all.iter().any(|l| l.contains("Loading")), "{all:?}");
    }

    #[test]
    fn an_error_shows_the_plain_message_never_loading() {
        let v = View::from_result(Err("printing service refused the request (IPP status 0x0500)".into()));
        let spec = spec(Some(&v));
        assert!(!labels(&spec).iter().any(|l| l.contains("Loading")));
        assert_eq!(spec.groups[0].footer.as_deref(), Some(NOT_RUNNING));
    }

    #[test]
    fn only_before_the_first_read_lands_the_page_says_loading() {
        assert!(labels(&spec(None)).iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn a_slow_or_panicking_read_is_an_error_not_a_wait_forever() {
        let slow = bounded(std::time::Duration::from_millis(20), || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            Ok::<u8, String>(1)
        });
        assert!(slow.is_err());
        let panics = bounded(std::time::Duration::from_secs(2), || -> Result<u8, String> { panic!("boom") });
        assert!(panics.is_err());
        assert_eq!(bounded(std::time::Duration::from_secs(2), || Ok::<u8, String>(3)), Ok(3));
    }

    #[test]
    fn a_pending_edit_shows_a_working_row_on_top() {
        let v = View::from_result(Ok(vec![]));
        assert!(!labels(&spec_pending(Some(&v), false)).iter().any(|l| l.contains("Working")));
        let pending = labels(&spec_pending(Some(&v), true));
        assert!(pending[0].contains("Working"), "{pending:?}");
        assert!(pending.iter().any(|l| l.starts_with("Add Printer")), "{pending:?}");
    }

    #[test]
    fn a_panicking_edit_lands_as_an_error() {
        match run_edit(|| panic!("boom")) {
            Landed::Edited(Err(_)) => {}
            _ => panic!("a panic must become an error"),
        }
        assert!(matches!(run_edit(|| Ok(None)), Landed::Edited(Ok(None))));
    }

    #[test]
    fn the_scanner_list_is_reused_and_concurrent_loads_share_one_read() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let slot: std::sync::Arc<std::sync::Mutex<Option<(std::time::Instant, u32)>>> = Default::default();
        let reads = std::sync::Arc::new(AtomicUsize::new(0));
        let hour = std::time::Duration::from_secs(3600);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (slot, reads) = (slot.clone(), reads.clone());
                std::thread::spawn(move || {
                    cached(&slot, hour, || {
                        reads.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        7
                    })
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), 7);
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        // an old answer is read again
        assert_eq!(cached(&slot, std::time::Duration::ZERO, || 9), 9);
    }

    #[test]
    fn only_device_addresses_the_scheduler_takes_are_added() {
        assert_eq!(finish(&Purpose::ComputerName, "ipp://x").as_ref().err(), None, "other purposes are left to their page");
        assert!(finish(&Purpose::PrinterAddress, "ftp://x").is_err());
    }
}
