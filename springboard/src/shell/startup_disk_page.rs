//! Settings > General > Startup Disk: the systems this computer can start from, and the one it starts from.
//!
//! The list comes from `bifrost-privd` (`startup_disk.list`, the firmware boot table read with `efibootmgr`). Choosing a
//! system changes what the whole computer starts, so it needs an administrator's approval first: the row queues the
//! shell's password sheet (`auth_sheet.rs`, the same sheet an app's `local_auth` request uses, checked against the
//! account password) and only an approved answer reaches privd (`startup_disk.set`). A wrong password, a cancel or a
//! locked sheet changes nothing. A system the firmware marks inactive is listed but cannot be chosen.
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use super::services::local_auth::Failure;

/// The name the approval request goes under in the password sheet queue (`local_auth::queue`).
pub const APP: &str = "settings.startup_disk";

/// One entry of the firmware boot table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    /// four hex digits, the firmware's boot number
    pub id: String,
    pub label: String,
    /// the firmware can start it
    pub active: bool,
    /// this start came from it
    pub current: bool,
    /// the next start only is set to it
    pub next: bool,
    /// the first system in the boot order
    pub default: bool,
}

/// The boot table as privd answered, or why it could not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub disks: Vec<Disk>,
    pub error: Option<String>,
}

/// What a row on the page does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// start from the entry with this id from now on (after approval)
    Choose(String),
}

impl View {
    /// Reads the boot table through privd. A computer that did not start from UEFI has none: that is shown, not hidden.
    pub fn load() -> View {
        match crate::privd_client::connect().startup_disks() {
            Ok(rows) => View {
                disks: rows.into_iter().map(|(id, label, active, current, next, default)| Disk { id, label, active, current, next, default }).collect(),
                error: None,
            },
            Err(e) => View { disks: Vec::new(), error: Some(e.message) },
        }
    }
}

/// What a row says about its system.
fn state_label(d: &Disk) -> &'static str {
    match (d.current, d.next, d.active) {
        (_, _, false) => "Not Available",
        (true, _, _) => "Started From",
        (_, true, _) => "Next Start",
        _ => "",
    }
}

/// The page's groups: the systems in boot order with a check on the one that starts by default. While an approval is
/// waiting on the password sheet (`pending` is the id) that row says so and nothing else can be chosen.
pub fn groups(v: &View, pending: Option<&str>) -> Vec<Group> {
    if let Some(e) = &v.error {
        return vec![Group::new(vec![Item::new("Startup Disk").value("Unavailable").tint(Tint::Dim)]).footer(&format!("The list of systems could not be read: {e}"))];
    }
    if v.disks.is_empty() {
        return vec![Group::new(vec![Item::new("No Startup Disks").tint(Tint::Dim)]).footer("The firmware lists no system to start from.")];
    }
    let rows = v
        .disks
        .iter()
        .map(|d| {
            let waiting = pending == Some(d.id.as_str());
            let value = if waiting { "Waiting for Approval" } else { state_label(d) };
            let item = Item::new(d.label.as_str()).value(value);
            if d.active && pending.is_none() {
                item.check(d.default, Action::StartupDisk(Edit::Choose(d.id.clone())))
            } else {
                item.tint(if d.active { Tint::Plain } else { Tint::Dim })
            }
        })
        .collect();
    vec![Group::new(rows)
        .header("START THIS COMPUTER FROM")
        .footer("The checked system starts when this computer turns on. Changing it asks for an administrator password.")]
}

/// The page, with a placeholder row until the first read lands.
pub fn spec(view: Option<&View>, pending: Option<&str>) -> Spec {
    let groups = match view {
        Some(v) => groups(v, pending),
        None => vec![Group::new(vec![Item::new("Startup Disk").value("Loading…").tint(Tint::Dim)])],
    };
    Spec { id: PageId::StartupDisk, title: "Startup Disk".into(), large: false, groups }
}

/// The system a Choose edit names, when it can be chosen: it is listed, the firmware can start it, and it is not
/// already the default.
pub fn choose<'a>(v: &'a View, edit: &Edit) -> Result<&'a Disk, String> {
    let Edit::Choose(id) = edit;
    let disk = v.disks.iter().find(|d| d.id == *id).ok_or("That system is no longer listed.")?;
    if !disk.active {
        return Err(format!("{} cannot be started by this computer.", disk.label));
    }
    if disk.default {
        return Err(format!("This computer already starts from {}.", disk.label));
    }
    Ok(disk)
}

/// The line the password sheet shows for the change.
pub fn reason(d: &Disk) -> String {
    format!("Enter your password to start this computer from “{}”.", d.label)
}

/// What the password sheet's answer means for the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// approved: privd may set the entry
    Approved,
    /// the person cancelled: nothing to say
    Cancelled,
    /// not approved, with the reason to show
    Refused(String),
}

pub fn outcome(answer: Result<(), Failure>) -> Outcome {
    match answer {
        Ok(()) => Outcome::Approved,
        Err(Failure::UserCancel) => Outcome::Cancelled,
        Err(Failure::AuthenticationFailed) => Outcome::Refused("The password was not right, so the startup disk was not changed.".into()),
        Err(Failure::PasscodeNotSet) => Outcome::Refused("This account has no password, so the change cannot be approved.".into()),
        Err(_) => Outcome::Refused("The approval did not finish, so the startup disk was not changed.".into()),
    }
}

/// Makes `id` the default start through privd.
pub fn apply(id: &str) -> Result<(), String> {
    crate::privd_client::connect().set_startup_disk(id, false).map_err(|e| e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(id: &str, label: &str, active: bool, current: bool, default: bool) -> Disk {
        Disk { id: id.into(), label: label.into(), active, current, next: false, default }
    }

    fn view() -> View {
        View { disks: vec![disk("0001", "BifrOSt", true, true, true), disk("0003", "Rescue", true, false, false), disk("0004", "Old Disk", false, false, false)], error: None }
    }

    #[test]
    fn the_default_system_is_checked_and_each_usable_row_chooses_itself() {
        let g = groups(&view(), None);
        assert_eq!(g[0].items.len(), 3);
        assert_eq!(g[0].items[0].value, "Started From");
        assert_eq!(g[0].items[1].action, Action::StartupDisk(Edit::Choose("0003".into())));
        assert_eq!(g[0].items[2].value, "Not Available");
        assert_eq!(g[0].items[2].action, Action::None, "an inactive system cannot be chosen");
    }

    #[test]
    fn nothing_can_be_chosen_while_an_approval_is_waiting() {
        let g = groups(&view(), Some("0003"));
        assert_eq!(g[0].items[1].value, "Waiting for Approval");
        assert!(g[0].items.iter().all(|i| i.action == Action::None));
    }

    #[test]
    fn a_read_that_failed_says_why_and_offers_nothing() {
        let g = groups(&View { disks: vec![], error: Some("efibootmgr failed".into()) }, None);
        assert_eq!(g[0].items[0].action, Action::None);
        assert!(g[0].footer.as_deref().unwrap().contains("efibootmgr failed"));
        assert_eq!(spec(None, None).groups[0].items[0].value, "Loading…");
    }

    #[test]
    fn only_a_listed_usable_other_system_can_be_chosen() {
        let v = view();
        assert_eq!(choose(&v, &Edit::Choose("0003".into())).unwrap().label, "Rescue");
        assert!(choose(&v, &Edit::Choose("0001".into())).is_err(), "already the default");
        assert!(choose(&v, &Edit::Choose("0004".into())).is_err(), "inactive");
        assert!(choose(&v, &Edit::Choose("0009".into())).is_err(), "not listed");
    }

    #[test]
    fn only_an_approved_answer_lets_privd_set_the_entry() {
        assert_eq!(outcome(Ok(())), Outcome::Approved);
        assert_eq!(outcome(Err(Failure::UserCancel)), Outcome::Cancelled);
        for f in [Failure::AuthenticationFailed, Failure::PasscodeNotSet, Failure::SystemCancel, Failure::NotInteractive] {
            assert!(matches!(outcome(Err(f)), Outcome::Refused(_)), "{f:?}");
        }
    }

    #[test]
    fn the_sheet_names_the_system() {
        assert!(reason(&disk("0003", "Rescue", true, false, false)).contains("Rescue"));
    }
}
