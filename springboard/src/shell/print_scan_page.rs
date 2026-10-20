//! Settings > Print & Scan: the printers the local printing service knows (`bifrost_ui::cups`, IPP over the local
//! scheduler), the default printer, and adding a network or USB printer by its device address. The list is read on
//! every build of the page, so a change shows at once. The printing service being down is shown as the page's
//! footer, not as a failure of the whole pane.
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
    pub fn load() -> View {
        let v = View::from_result(Client::local().printers().map_err(|e| e.to_string()))
            .with_scanners(scanner::list().map_err(|e| e.to_string()));
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
    let groups = match v {
        Some(v) => groups(v),
        None => vec![Group::new(vec![Item::new("Printers").value("Loading…").tint(Tint::Dim)])],
    };
    Spec { id: PageId::PrintScan, title: "Print & Scan".into(), large: false, groups }
}

/// Applies a Print & Scan edit: the default printer changes now (`None`), the address prompt opens (`Some`).
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

    #[test]
    fn only_device_addresses_the_scheduler_takes_are_added() {
        assert_eq!(finish(&Purpose::ComputerName, "ipp://x").as_ref().err(), None, "other purposes are left to their page");
        assert!(finish(&Purpose::PrinterAddress, "ftp://x").is_err());
    }
}
