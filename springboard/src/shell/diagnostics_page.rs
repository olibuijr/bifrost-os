//! Settings › Privacy › Analytics & Diagnostics as data (see `diag`).

use super::diag::{self, CrashEntry};
use crate::install_log::Failure;
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, Command, PageId};

/// What the page shows besides the crash list.
pub struct PageState<'a> {
    pub crashes: &'a [CrashEntry],
    /// failed app and system installs, newest first (`install_log::recent`)
    pub installs: &'a [Failure],
    /// zone offset in seconds east of UTC, for the dates
    pub offset: i32,
    /// "Shared with the developer.", the last error, or empty
    pub note: &'a str,
    /// an upload is running
    pub sharing: bool,
}

/// Newest crashes listed; older ones stay in the log files.
pub const MAX_ROWS: usize = 30;

pub fn spec(st: &PageState) -> Spec {
    let rows: Vec<Item> = if st.crashes.is_empty() {
        vec![Item::new("No Crash Logs").tint(Tint::Dim)]
    } else {
        st.crashes.iter().take(MAX_ROWS).map(|c| Item::new(diag::date_text(c.unix, st.offset)).value(c.signature.clone()).action(Action::OpenFile(c.file.clone()))).collect()
    };
    let install_rows: Vec<Item> = if st.installs.is_empty() {
        vec![Item::new("No Install Failures").tint(Tint::Dim)]
    } else {
        st.installs.iter().take(MAX_ROWS).map(|f| Item::new(diag::date_text(f.unix, st.offset)).value(f.message.clone())).collect()
    };
    let have = !st.crashes.is_empty();
    let share = if st.sharing { Item::new("Sharing…").tint(Tint::Dim) } else { Item::new("Share with Developer").tint(Tint::Blue).action(Action::Run(Command::ShareDiagnostics)) };
    let delete = Item::new("Delete All Crash Logs").tint(if have { Tint::Red } else { Tint::Dim }).action(Action::Run(Command::DeleteCrashLogs));
    let mut groups = vec![
        Group::new(rows).header("CRASH LOGS").footer("Tap an entry to read the log it came from."),
        Group::new(install_rows).header("INSTALL FAILURES").footer("Apps and system updates that failed to install, newest first."),
        Group::new(vec![share]).footer("Sends the build, crash logs and heartbeat to the developer's update server on your network. Nothing leaves it."),
        Group::new(vec![delete]),
    ];
    if !st.note.is_empty() {
        groups.push(Group::new(vec![]).footer(st.note));
    }
    Spec { id: PageId::Diagnostics, title: "Analytics & Diagnostics".into(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn crash(unix: i64) -> CrashEntry {
        CrashEntry { unix, signature: "a.rs:1 boom".into(), file: PathBuf::from("/d/crash.log") }
    }

    fn install(unix: i64, message: &str) -> Failure {
        Failure { unix, what: "app install".into(), message: message.into() }
    }

    #[test]
    fn empty_page_has_nothing_to_delete() {
        let s = spec(&PageState { crashes: &[], installs: &[], offset: 0, note: "", sharing: false });
        assert_eq!(s.groups[0].items[0].tint, Tint::Dim);
        assert_eq!(s.groups[1].items[0].tint, Tint::Dim);
        assert_eq!(s.groups[3].items[0].tint, Tint::Dim);
    }

    #[test]
    fn crashes_open_their_log_and_share_is_offered() {
        let cs = [crash(100), crash(50)];
        let s = spec(&PageState { crashes: &cs, installs: &[], offset: 0, note: "Shared.", sharing: false });
        assert_eq!(s.groups[0].items.len(), 2);
        assert_eq!(s.groups[0].items[0].action, Action::OpenFile(PathBuf::from("/d/crash.log")));
        assert_eq!(s.groups[2].items[0].action, Action::Run(Command::ShareDiagnostics));
        assert_eq!(s.groups[3].items[0].tint, Tint::Red);
        assert_eq!(s.groups.len(), 5);
    }

    #[test]
    fn install_failures_are_listed_with_their_message() {
        let is = [install(200, "pkg checksum mismatch"), install(100, "disk full")];
        let s = spec(&PageState { crashes: &[], installs: &is, offset: 0, note: "", sharing: false });
        assert_eq!(s.groups[1].header.as_deref(), Some("INSTALL FAILURES"));
        assert_eq!(s.groups[1].items.len(), 2);
        assert_eq!(s.groups[1].items[0].value, "pkg checksum mismatch");
        assert_eq!(s.groups[1].items[0].action, Action::None);
    }

    #[test]
    fn sharing_disables_the_row() {
        let s = spec(&PageState { crashes: &[], installs: &[], offset: 0, note: "", sharing: true });
        assert_eq!(s.groups[2].items[0].action, Action::None);
    }
}
