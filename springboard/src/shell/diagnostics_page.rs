//! Settings › Privacy › Analytics & Diagnostics as data (see `diag`).

use super::diag::{self, CrashEntry};
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, Command, PageId};

/// What the page shows besides the crash list.
pub struct PageState<'a> {
    pub crashes: &'a [CrashEntry],
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
    let have = !st.crashes.is_empty();
    let share = if st.sharing { Item::new("Sharing…").tint(Tint::Dim) } else { Item::new("Share with Developer").tint(Tint::Blue).action(Action::Run(Command::ShareDiagnostics)) };
    let delete = Item::new("Delete All Crash Logs").tint(if have { Tint::Red } else { Tint::Dim }).action(Action::Run(Command::DeleteCrashLogs));
    let mut groups = vec![
        Group::new(rows).header("CRASH LOGS").footer("Tap an entry to read the log it came from."),
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

    #[test]
    fn empty_page_has_nothing_to_delete() {
        let s = spec(&PageState { crashes: &[], offset: 0, note: "", sharing: false });
        assert_eq!(s.groups[0].items[0].tint, Tint::Dim);
        assert_eq!(s.groups[2].items[0].tint, Tint::Dim);
    }

    #[test]
    fn crashes_open_their_log_and_share_is_offered() {
        let cs = [crash(100), crash(50)];
        let s = spec(&PageState { crashes: &cs, offset: 0, note: "Shared.", sharing: false });
        assert_eq!(s.groups[0].items.len(), 2);
        assert_eq!(s.groups[0].items[0].action, Action::OpenFile(PathBuf::from("/d/crash.log")));
        assert_eq!(s.groups[1].items[0].action, Action::Run(Command::ShareDiagnostics));
        assert_eq!(s.groups[2].items[0].tint, Tint::Red);
        assert_eq!(s.groups.len(), 4);
    }

    #[test]
    fn sharing_disables_the_row() {
        let s = spec(&PageState { crashes: &[], offset: 0, note: "", sharing: true });
        assert_eq!(s.groups[1].items[0].action, Action::None);
    }
}
