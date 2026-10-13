//! Settings > Print & Scan: the printers the local printing service knows (`bifrost_ui::cups`, IPP over the local
//! scheduler), the default printer, and adding a network or USB printer by its device address. The list is read on
//! every build of the page, so a change shows at once. The printing service being down is shown as the page's
//! footer, not as a failure of the whole pane.
use super::forms::{Prompt, Purpose};
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use bifrost_ui::cups::{self, Client, Printer, PrinterState};

/// What a row on the page does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// makes this queue the default printer
    SetDefault(String),
    /// opens the address prompt for a new printer
    Add,
}

/// The printers as the scheduler has them now, or why they could not be read.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub printers: Vec<Printer>,
    pub error: Option<String>,
}

impl View {
    pub fn load() -> View {
        View::from_result(Client::local().printers().map_err(|e| e.to_string()))
    }

    pub fn from_result(r: Result<Vec<Printer>, String>) -> View {
        match r {
            Ok(printers) => View { printers, error: None },
            Err(e) => View { printers: Vec::new(), error: Some(e) },
        }
    }

    fn names(&self) -> Vec<String> {
        self.printers.iter().map(|p| p.name.clone()).collect()
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

/// The page's groups: the printers with the default one checked, then the add row.
pub fn groups(v: &View) -> Vec<Group> {
    let mut out = Vec::new();
    match &v.error {
        Some(e) => out.push(Group::new(vec![Item::new("No Printing Service").tint(Tint::Dim)]).footer(&format!("Printing is not available: {e}.")).header("PRINTERS")),
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
    out
}

/// The whole Print & Scan page.
pub fn spec(v: &View) -> Spec {
    Spec { id: PageId::PrintScan, title: "Print & Scan".into(), large: false, groups: groups(v) }
}

/// Applies a Print & Scan edit: the default printer changes now (`None`), the address prompt opens (`Some`).
pub fn apply(edit: &Edit) -> Result<Option<Prompt>, String> {
    match edit {
        Edit::SetDefault(name) => Client::local().set_default(name).map(|_| None).map_err(|e| e.to_string()),
        Edit::Add => Ok(Some(Prompt::printer_address())),
    }
}

/// A queue name from a device address: its host (or the USB or socket target) with characters the scheduler does
/// not take replaced by hyphens. `taken` names are numbered (`-2`, `-3`) so an existing queue is never replaced.
pub fn queue_name(device_uri: &str, taken: &[String]) -> String {
    let rest = device_uri.split_once("://").map_or(device_uri, |(_, r)| r);
    let host: String = rest.split(|c| c == '/' || c == ':').next().unwrap_or("").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(60).collect();
    let base = if host.trim_matches('-').is_empty() { "Printer".to_string() } else { host.trim_matches('-').to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Finishes the address prompt: adds the queue through the scheduler. Other purposes are not this page's.
pub fn finish(purpose: &Purpose, text: &str) -> Result<(), String> {
    if *purpose != Purpose::PrinterAddress {
        return Ok(());
    }
    let device = text.trim();
    if !cups::valid_device_uri(device) {
        return Err(format!("unsupported device address {device:?}"));
    }
    let mut c = Client::local();
    let taken = View::from_result(c.printers().map_err(|e| e.to_string())).names();
    let name = queue_name(device, &taken);
    c.add_driverless_printer(&name, device).map_err(|e| e.to_string())
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
    fn no_printing_service_shows_why_and_offers_no_add_row() {
        let v = View::from_result(Err("printing service unreachable: refused".into()));
        let g = groups(&v);
        assert_eq!(g.len(), 1);
        assert!(g[0].footer.as_deref().is_some_and(|f| f.contains("refused")));
    }

    #[test]
    fn an_empty_list_still_offers_to_add_a_printer() {
        let g = groups(&View::from_result(Ok(vec![])));
        assert_eq!(g[0].items[0].label, "No Printers");
        assert_eq!(g[1].items[0].action, Action::PrintScan(Edit::Add));
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
    fn only_device_addresses_the_scheduler_takes_are_added() {
        assert_eq!(finish(&Purpose::ComputerName, "ipp://x").as_ref().err(), None, "other purposes are left to their page");
        assert!(finish(&Purpose::PrinterAddress, "ftp://x").is_err());
    }
}
