//! The Backup and Software Update pages as data (see `backup` and `update`).

use super::backup;
use super::list::{Body, Group, Item, Spec, Tint};
use super::pages::Toggle;
use super::pages::{Action, Command, PageId, Pick, BUILD};
use super::storage_page::Bar;
use super::update::{Manifest, Source, State, Updater};
use crate::gfx::pack;

/// The download progress bar of the Software Update page.
pub fn progress_bar(u: &Updater) -> Bar {
    if u.source() == Source::Pacman {
        // pacman reports no progress to us
        return Bar { caption: "Installing with pacman…".into(), total: 100, segments: vec![] };
    }
    let pct = (u.fraction() * 100.0) as u64;
    Bar { caption: format!("Downloading {pct}%"), total: 100, segments: vec![(pct, pack(10, 132, 255, 255))] }
}

/// What the pages show beyond the static build info.
pub struct PageState<'a> {
    pub updater: &'a Updater,
    /// Automatic Updates: download offered updates in the background
    pub auto_update: bool,
    /// Back Up Before Updates: back up the data before an update installs
    pub backup_before_update: bool,
    /// "N files backed up", the last error, or empty
    pub backup_note: &'a str,
    /// unix time of the newest backup in the chosen destination
    pub latest_backup: Option<u64>,
    /// the chosen backup disk's mount point, or "" for this computer
    pub destination: &'a str,
}

pub fn spec(id: PageId, st: &PageState) -> Spec {
    match id {
        PageId::Backup => backup_spec(st),
        _ => update_spec(st),
    }
}

/// The places a backup can go: This Computer (the data folder, path "") and each removable disk mounted now, with
/// its mount point as the path. Listed in the order the disks are mounted.
pub fn destination_choices() -> Vec<(String, String)> {
    let mut out = vec![("This Computer".to_string(), String::new())];
    for v in crate::files::volumes::mounted() {
        let removable = ["/media/", "/run/media/", "/mnt/"].iter().any(|p| v.mount.to_string_lossy().starts_with(p));
        if removable {
            out.push((v.name.clone(), v.mount.to_string_lossy().into_owned()));
        }
    }
    out
}

fn backup_spec(st: &PageState) -> Spec {
    let latest = st.latest_backup.map_or("None".to_string(), |t| format!("{} min ago", (now().saturating_sub(t)) / 60));
    let restore = Item::new("Restore Latest Backup").tint(if st.latest_backup.is_some() { Tint::Blue } else { Tint::Dim }).action(Action::Run(Command::RestoreBackup));
    let destinations: Vec<Item> = destination_choices()
        .into_iter()
        .enumerate()
        .map(|(i, (label, path))| Item::new(label).check(path == st.destination, Action::Pick(Pick::BackupDestination(i))))
        .collect();
    let mut groups = vec![
        Group::new(destinations).header("BACKUP DESTINATION").footer("Backups are kept in the chosen place. A removable disk must stay connected to back up or restore."),
        Group::new(vec![Item::new("Back Up Now").tint(Tint::Blue).action(Action::Run(Command::BackUpNow)), Item::new("Latest Backup").value(latest)])
            .footer(&format!("Saves Notes, Contacts, Calendar, Reminders, settings and notifications to one file in {}/. `tab backup` copies it to a computer.", backup::BACKUP_DIR)),
        Group::new(vec![restore]).footer("Replaces that data with the backup and restarts SpringBoard. The backup is checked first; a damaged one is refused."),
        Group::new(vec![Item::new("Delete All Backups").tint(if st.latest_backup.is_some() { Tint::Red } else { Tint::Dim }).action(Action::Run(Command::DeleteBackups))])
            .footer("Removes every backup file. Your data is not changed."),
    ];
    if !st.backup_note.is_empty() {
        groups.push(Group::new(vec![]).footer(st.backup_note));
    }
    Spec { id: PageId::Backup, title: "Backup".into(), large: false, groups }
}

fn update_spec(st: &PageState) -> Spec {
    let u = st.updater;
    let pacman = u.source() == Source::Pacman;
    let mut info = vec![Item::new(if pacman { "BifrOSt" } else { "SpringBoard" }).value(BUILD.version), Item::new("Build").value(BUILD.git), Item::new("Built").value(BUILD.date)];
    if let (true, Some(v)) = (pacman, u.installed()) {
        info.push(Item::new("Package").value(v));
    }
    let mut groups = vec![Group::new(info)];
    let mut action = vec![];
    let footer;
    match &u.state {
        State::Idle => {
            action.push(Item::new("Check for Update").tint(Tint::Blue).action(Action::Run(Command::CheckUpdate)));
            footer = if pacman { "Looks in the Arch package repositories (pacman).".into() } else { format!("Looks at {}.", u.url()) };
        }
        State::Checking => {
            action.push(Item::new("Checking for Update…").tint(Tint::Dim));
            footer = if pacman { "Asking pacman.".into() } else { format!("Asking {}.", u.url()) };
        }
        State::UpToDate => {
            action.push(Item::new("Check for Update").tint(Tint::Blue).action(Action::Run(Command::CheckUpdate)));
            footer = if pacman { "BifrOSt is up to date." } else { "SpringBoard is up to date." }.into();
        }
        State::Available(m) => {
            groups.push(card(m));
            action.push(Item::new("Download and Install").tint(Tint::Blue).action(Action::Run(Command::DownloadUpdate)));
            footer = if pacman {
                "Installing runs pacman -Syu, which upgrades the whole system together with the shell.".into()
            } else if st.auto_update { "BifrOSt downloads the update by itself; you decide when to install it.".into() } else { "The download is checked against its SHA-256 before it is staged.".into() };
        }
        State::Downloading => {
            groups.push(Group::new(vec![Item { body: Body::Bar(progress_bar(u)), ..Item::new("") }]).header("UPDATE"));
            footer = if pacman { "pacman verifies package signatures itself.".into() } else { "The download is checked against its SHA-256 before it is staged.".into() };
        }
        State::Staged(m) => {
            if m.version != "staged" {
                groups.push(card(m));
            }
            action.push(Item::new("Install Now").tint(Tint::Blue).action(Action::Run(Command::InstallUpdate)));
            footer = if pacman { "The update is installed. BifrOSt restarts into the new version.".into() } else { "The update is verified and staged. It starts on the next launch and rolls back by itself if it fails within a minute.".into() };
        }
        State::Failed(e) => {
            action.push(Item::new("Check for Update").tint(Tint::Blue).action(Action::Run(Command::CheckUpdate)));
            footer = format!("Update failed: {e}");
        }
    }
    if !action.is_empty() {
        groups.push(Group::new(action).footer(&footer));
    } else {
        groups.push(Group::new(vec![]).footer(&footer));
    }
    if pacman {
        // The guard's rollback-start op is granted through sudo like install (packaging/arch/README.md).
        groups.push(Group::new(vec![Item::new("Roll Back to Previous System").tint(Tint::Red).action(Action::Run(Command::RollbackSystem))]).footer("Returns BifrOSt to the version before the last update. Needs a previous version to have been kept."));
    }
    groups.push(Group::new(vec![Item::new("Automatic Updates").switch(st.auto_update, Action::Toggle(Toggle::AutoUpdate))]).footer("BifrOSt downloads system updates in the background and asks before installing them. App Store apps update by themselves while they are closed."));
    groups.push(Group::new(vec![Item::new("Back Up Before Updates").switch(st.backup_before_update, Action::Toggle(Toggle::BackupBeforeUpdate))]).footer("Saves a backup of your data before an update is installed. If the backup fails, the update is not installed."));
    Spec { id: PageId::SoftwareUpdate, title: "Software Update".into(), large: false, groups }
}

/// The bifrost 14 update card: name and version, size, release notes.
fn card(m: &Manifest) -> Group {
    let size = if m.size > 0 { format!("{} MB", (m.size + 999_999) / 1_000_000) } else { String::new() };
    let g = Group::new(vec![Item::new(format!("BifrOSt {}", m.version)).value(size)]);
    let latest = m.changelog.iter().find(|r| !r.notes.is_empty());
    match (latest, &m.notes) {
        (Some(r), _) => g.header(&format!("WHAT'S NEW IN {}", r.version)).footer(&r.notes.iter().map(|n| format!("\u{2022} {n}")).collect::<Vec<_>>().join(" ")),
        (None, Some(n)) => g.footer(n),
        (None, None) => g.footer(&format!("Version {}{}.", m.version, m.build.as_ref().map_or(String::new(), |b| format!(" ({b})")))),
    }
}

fn now() -> u64 {
    crate::shell::clock::tz::unix_now() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::list::Accessory;

    #[test]
    fn update_page_offers_the_step_for_each_state() {
        let mut u = Updater::new(std::env::temp_dir().join("sb-nonexistent-dir"), None);
        let st = |u: &Updater| spec(PageId::SoftwareUpdate, &PageState { updater: u, auto_update: true, backup_before_update: true, backup_note: "", latest_backup: None, destination: "" });
        assert_eq!(st(&u).groups[1].items[0].action, Action::Run(Command::CheckUpdate));
        u.state = State::Checking;
        assert_eq!(st(&u).groups[1].items[0].action, Action::None);
        u.state = State::Downloading;
        assert!(matches!(st(&u).groups[1].items[0].body, Body::Bar(_)));
        let m = Manifest { version: "0.2.0".into(), build: None, sha256: String::new(), size: 12_300_000, url: String::new(), notes: Some("Faster.".into()), changelog: vec![] };
        u.state = State::Available(m.clone());
        let s = st(&u);
        assert_eq!((s.groups[1].items[0].label.as_str(), s.groups[1].items[0].value.as_str()), ("BifrOSt 0.2.0", "13 MB"));
        assert_eq!(s.groups[2].items[0].label, "Download and Install");
        u.state = State::Staged(m);
        assert_eq!(st(&u).groups[2].items[0].action, Action::Run(Command::InstallUpdate));
        assert_eq!(st(&u).groups[2].items[0].label, "Install Now");
        let last = st(&u).groups.len() - 2;
        assert_eq!(st(&u).groups[last].items[0].action, Action::Toggle(Toggle::AutoUpdate));
        assert_eq!(st(&u).groups[last + 1].items[0].action, Action::Toggle(Toggle::BackupBeforeUpdate));
    }

    #[test]
    fn update_card_prefers_the_changelog_over_the_notes_string() {
        let rel = crate::shell::store::api::Release { version: "0.2.0".into(), date: String::new(), notes: vec!["Faster".into(), "Fixed".into()] };
        let m = Manifest { version: "0.2.0".into(), build: None, sha256: String::new(), size: 1, url: String::new(), notes: Some("old".into()), changelog: vec![rel] };
        let g = card(&m);
        assert_eq!(g.footer.as_deref(), Some("\u{2022} Faster \u{2022} Fixed"));
        assert_eq!(card(&Manifest { changelog: vec![], ..m }).footer.as_deref(), Some("old"));
    }

    #[test]
    fn the_destination_group_marks_the_chosen_place_and_starts_with_this_computer() {
        let u = Updater::new(std::path::PathBuf::from("/nonexistent"), None);
        let s = spec(PageId::Backup, &PageState { updater: &u, auto_update: true, backup_before_update: true, backup_note: "", latest_backup: None, destination: "" });
        let first = &s.groups[0];
        assert_eq!(first.items[0].label, "This Computer");
        assert_eq!(first.items[0].accessory, Accessory::Check(true), "the data folder is the default");
        assert_eq!(first.items[0].action, Action::Pick(Pick::BackupDestination(0)));
        let chosen = spec(PageId::Backup, &PageState { updater: &u, auto_update: true, backup_before_update: true, backup_note: "", latest_backup: None, destination: "/run/media/u/USB" });
        assert_eq!(chosen.groups[0].items[0].accessory, Accessory::Check(false));
    }

    #[test]
    fn delete_backups_is_offered_and_dim_without_a_backup() {
        let u = Updater::new(std::env::temp_dir().join("sb-nonexistent-dir"), None);
        let none = spec(PageId::Backup, &PageState { updater: &u, auto_update: true, backup_before_update: true, backup_note: "", latest_backup: None, destination: "" });
        // Groups: destination, backup, restore, delete.
        assert_eq!(none.groups[3].items[0].action, Action::Run(Command::DeleteBackups));
        assert_eq!(none.groups[3].items[0].tint, Tint::Dim);
        let some = spec(PageId::Backup, &PageState { updater: &u, auto_update: true, backup_before_update: true, backup_note: "", latest_backup: Some(1), destination: "" });
        assert_eq!(some.groups[3].items[0].tint, Tint::Red);
    }

    #[test]
    fn restore_is_dim_without_a_backup() {
        let u = Updater::new(std::env::temp_dir().join("sb-nonexistent-dir"), None);
        let s = spec(PageId::Backup, &PageState { updater: &u, auto_update: true, backup_before_update: true, backup_note: "", latest_backup: None, destination: "" });
        assert_eq!(s.groups[2].items[0].tint, Tint::Dim);
    }
}
