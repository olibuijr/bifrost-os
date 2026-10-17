//! Settings > Sharing, the part that changes the computer itself: the computer name and the three sharing services
//! (Remote Login, File Sharing, Screen Sharing, Printer Sharing). The name is read from `/etc/machine-info` (the pretty host name
//! `hostnamectl` writes) and each service's state from `systemctl is-enabled`. Changes go to `bifrost-privd`
//! (`system.set_names`, `service.set`), which validates them and runs the fixed command as root.
//! Nearby Share and AirPlay & Handoff stay on the same page, below these groups (`pages::spec`).
use super::forms::{Prompt, Purpose};
use super::list::{Group, Item};
use super::pages::Action;
use crate::privd::{accounts, screen_sharing};

/// The sharing services the page switches, as (privd id, row label), in the order they are listed.
pub const SERVICES: [(&str, &str); 4] = [("remote_login", "Remote Login"), ("file_sharing", "File Sharing"), ("screen_sharing", "Screen Sharing"), ("printer_sharing", "Printer Sharing")];

/// What a row on the Sharing page does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// opens the Computer Name prompt
    ComputerName,
    /// starts (`on`) or stops one service by its privd id
    Service { id: &'static str, on: bool },
    /// opens the prompt for the account that may sign in to Screen Sharing
    ScreenSharingAccount,
}

/// The computer name and the service states as the system has them now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub computer: String,
    pub services: Vec<(&'static str, bool)>,
    /// the account Screen Sharing lets in (the allow list privd keeps), None until one is chosen
    pub screen_user: Option<String>,
}

/// The pretty host name in `/etc/machine-info` (`PRETTY_HOSTNAME=`), quotes removed; `None` when it is absent or empty.
pub fn pretty_name(machine_info: &str) -> Option<String> {
    machine_info
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_HOSTNAME="))
        .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|v| !v.is_empty())
}

/// True when `systemctl is-enabled` reports the unit enabled. A unit that is not installed is not enabled.
fn systemctl_enabled(unit: &str) -> bool {
    std::process::Command::new("systemctl").args(["is-enabled", "--quiet", unit]).status().map(|s| s.success()).unwrap_or(false)
}

impl View {
    pub fn load() -> View {
        let machine = std::fs::read_to_string("/etc/machine-info").unwrap_or_default();
        let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
        let mut v = View::from_text(&machine, &host, systemctl_enabled);
        v.screen_user = std::fs::read_to_string(screen_sharing::USERS).ok().and_then(|t| screen_sharing::user_from(&t));
        v
    }

    /// The view built from the files' text and a `unit -> enabled` answer.
    pub fn from_text(machine_info: &str, hostname: &str, enabled: impl Fn(&str) -> bool) -> View {
        let computer = pretty_name(machine_info).unwrap_or_else(|| hostname.trim().to_string());
        let services: Vec<(&'static str, bool)> = SERVICES.iter().map(|&(id, _)| (id, accounts::service_unit(id).is_some_and(|unit| enabled(unit)))).collect();
        View { computer, services, screen_user: None }
    }

    pub fn is_on(&self, id: &str) -> bool {
        self.services.iter().any(|&(s, on)| s == id && on)
    }

    /// Records a service the system has just started or stopped, so the page shows it before the next load lands.
    pub fn set_service(&mut self, id: &str, on: bool) {
        for (s, state) in self.services.iter_mut() {
            if *s == id {
                *state = on;
            }
        }
    }
}

/// The Computer Name row and the service switches. Each switch's action carries the state it asks for.
pub fn groups(v: &View) -> Vec<Group> {
    let name = Group::new(vec![Item::new("Computer Name").value(v.computer.as_str()).action(Action::Sharing(Edit::ComputerName))])
        .footer("The name other people see for this computer when it shares.");
    let items: Vec<Item> = SERVICES
        .iter()
        .map(|&(id, label)| {
            let on = v.is_on(id);
            Item::new(label).switch(on, Action::Sharing(Edit::Service { id, on: !on }))
        })
        .collect();
    let services = Group::new(items).footer("A service that is on starts now and whenever the computer starts. Turning it off stops it.");
    let account = Group::new(vec![Item::new("Screen Sharing Account").value(v.screen_user.as_deref().unwrap_or("Not Set")).action(Action::Sharing(Edit::ScreenSharingAccount))])
        .footer("Screen Sharing needs this account's name and password to connect, and the connection is encrypted. It is reachable only on the local network.");
    vec![name, services, account]
}

/// Applies a Sharing edit: the name opens its prompt (`Some`), a service switch runs now (`None`).
pub fn apply(edit: &Edit, v: &View) -> Result<Option<Prompt>, String> {
    match edit {
        Edit::ComputerName => Ok(Some(Prompt::computer_name(&v.computer))),
        // Screen Sharing never starts without an account that may sign in: the switch asks for it first
        Edit::Service { id: "screen_sharing", on: true } if v.screen_user.is_none() => Ok(Some(Prompt::screen_sharing_account("", true))),
        Edit::Service { id, on } => crate::privd_client::connect().set_service(id, *on).map(|_| None).map_err(|e| e.message),
        Edit::ScreenSharingAccount => Ok(Some(Prompt::screen_sharing_account(v.screen_user.as_deref().unwrap_or(""), false))),
    }
}

/// Finishes a Computer Name or Local Hostname prompt through privd. Other purposes are not this page's.
pub fn finish(purpose: &Purpose, text: &str) -> Result<(), String> {
    let c = crate::privd_client::connect();
    match purpose {
        Purpose::ComputerName => c.set_names(Some(text.trim()), None).map_err(|e| e.message),
        Purpose::LocalHostname => c.set_names(None, Some(text)).map_err(|e| e.message),
        Purpose::ScreenSharingAccount { start } => {
            c.screen_sharing_configure(text.trim()).map_err(|e| e.message)?;
            if *start {
                c.set_service("screen_sharing", true).map_err(|e| e.message)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// One folder the SMB server shares: its share name, its path, and whether network users may write to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedFolder {
    pub name: String,
    pub path: String,
    pub writable: bool,
}

/// The smb.conf section that shares a folder: `[name]` with its path, browseable, and read only unless writable.
pub fn share_section(folder: &SharedFolder) -> String {
    format!(
        "[{}]\n   path = {}\n   browseable = yes\n   read only = {}\n",
        folder.name,
        folder.path,
        if folder.writable { "no" } else { "yes" }
    )
}

/// The folder shares in an smb.conf text, in file order. `[global]` and `[homes]` are not folder shares, and a section
/// without a `path` is skipped. `read only = no` or `writable = yes` makes a folder writable.
pub fn shares_in_conf(conf: &str) -> Vec<SharedFolder> {
    let mut out: Vec<SharedFolder> = Vec::new();
    let mut current: Option<SharedFolder> = None;
    for raw in conf.lines() {
        let line = raw.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            out.extend(current.take());
            let name = name.trim();
            if !name.eq_ignore_ascii_case("global") && !name.eq_ignore_ascii_case("homes") {
                current = Some(SharedFolder { name: name.to_string(), path: String::new(), writable: false });
            }
            continue;
        }
        let Some(folder) = current.as_mut() else { continue };
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "path" => folder.path = value.to_string(),
            "read only" if value.eq_ignore_ascii_case("no") => folder.writable = true,
            "writable" | "writeable" if value.eq_ignore_ascii_case("yes") => folder.writable = true,
            _ => {}
        }
    }
    out.extend(current);
    out.retain(|f| !f.path.is_empty());
    out
}

/// The smb.conf text without the folder share called `name` (its section header and the lines under it, up to the next
/// header). `global` and `homes` are not folder shares and are never removed; a name with no section leaves the text as
/// it was.
pub fn without_share(conf: &str, name: &str) -> String {
    let name = name.trim();
    if name.eq_ignore_ascii_case("global") || name.eq_ignore_ascii_case("homes") {
        return conf.to_string();
    }
    let mut out = String::new();
    let mut skipping = false;
    for raw in conf.split_inclusive('\n') {
        if let Some(section) = raw.trim().strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            skipping = section.trim().eq_ignore_ascii_case(name);
        }
        if !skipping {
            out.push_str(raw);
        }
    }
    out
}

/// The smb.conf text with `folder` shared: any existing share of the same name is replaced, and the new section is
/// added at the end after a blank line. Other sections keep their order and text.
pub fn with_share(conf: &str, folder: &SharedFolder) -> String {
    if folder.name.trim().eq_ignore_ascii_case("global") || folder.name.trim().eq_ignore_ascii_case("homes") {
        return conf.to_string();
    }
    let mut out = without_share(conf, &folder.name);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&share_section(folder));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_share_added_to_the_conf_reads_back_with_the_others() {
        let docs = SharedFolder { name: "Docs".into(), path: "/srv/docs".into(), writable: false };
        let drop = SharedFolder { name: "Drop".into(), path: "/srv/drop".into(), writable: true };
        let conf = with_share("[global]\n   workgroup = HOME\n", &docs);
        let conf = with_share(&conf, &drop);
        assert!(conf.starts_with("[global]\n   workgroup = HOME\n\n[Docs]"));
        assert_eq!(shares_in_conf(&conf), vec![docs, drop]);
    }

    #[test]
    fn sharing_a_name_again_replaces_that_section_only() {
        let old = SharedFolder { name: "Docs".into(), path: "/srv/old".into(), writable: false };
        let new = SharedFolder { name: "docs".into(), path: "/srv/new".into(), writable: true };
        let conf = format!("[global]\n workgroup = HOME\n\n{}\n[Music]\n path = /srv/music\n", share_section(&old));
        let conf = with_share(&conf, &new);
        assert_eq!(
            shares_in_conf(&conf),
            vec![
                SharedFolder { name: "Music".into(), path: "/srv/music".into(), writable: false },
                new,
            ]
        );
        assert!(conf.contains("workgroup = HOME"));
    }

    #[test]
    fn unsharing_removes_one_section_and_keeps_the_rest() {
        let docs = SharedFolder { name: "Docs".into(), path: "/srv/docs".into(), writable: false };
        let conf = format!("[global]\n workgroup = HOME\n\n{}\n[Music]\n path = /srv/music\n", share_section(&docs));
        let after = without_share(&conf, "DOCS");
        assert_eq!(after, "[global]\n workgroup = HOME\n\n[Music]\n path = /srv/music\n");
        assert_eq!(without_share(&after, "missing"), after);
    }

    #[test]
    fn global_and_homes_are_never_removed_or_replaced() {
        let conf = "[global]\n workgroup = HOME\n[homes]\n path = /home\n";
        assert_eq!(without_share(conf, "global"), conf);
        assert_eq!(without_share(conf, "Homes"), conf);
        let bad = SharedFolder { name: "global".into(), path: "/x".into(), writable: true };
        assert_eq!(with_share(conf, &bad), conf);
    }

    #[test]
    fn a_folder_share_is_written_as_a_section_and_read_back() {
        let docs = SharedFolder { name: "Docs".into(), path: "/srv/docs".into(), writable: false };
        let drop = SharedFolder { name: "Drop".into(), path: "/srv/drop".into(), writable: true };
        let conf = format!("[global]\n   workgroup = HOME\n\n{}\n{}", share_section(&docs), share_section(&drop));
        assert_eq!(shares_in_conf(&conf), vec![docs, drop]);
    }

    #[test]
    fn global_homes_and_pathless_sections_are_not_folder_shares() {
        let conf = "[global]\n path = /x\n[homes]\n path = /home\n[empty]\n read only = no\n[Music]\nPath=/srv/music\nWritable = Yes\n";
        assert_eq!(
            shares_in_conf(conf),
            vec![SharedFolder { name: "Music".into(), path: "/srv/music".into(), writable: true }]
        );
    }

    fn off(_: &str) -> bool {
        false
    }

    #[test]
    fn the_pretty_name_is_read_without_quotes() {
        assert_eq!(pretty_name("PRETTY_HOSTNAME=\"Living Room\"\nICON_NAME=computer\n").as_deref(), Some("Living Room"));
        assert_eq!(pretty_name("PRETTY_HOSTNAME=tv\n").as_deref(), Some("tv"));
        assert_eq!(pretty_name("PRETTY_HOSTNAME=''\n"), None);
        assert_eq!(pretty_name("ICON_NAME=computer\n"), None);
    }

    #[test]
    fn the_name_falls_back_to_the_host_name() {
        let v = View::from_text("", "titan\n", off);
        assert_eq!(v.computer, "titan");
        assert!(v.services.iter().all(|(_, on)| !on));
    }

    #[test]
    fn services_report_their_unit_state() {
        let v = View::from_text("PRETTY_HOSTNAME=Hub\n", "", |unit| unit == "smb.service");
        assert_eq!(v.computer, "Hub");
        assert!(v.is_on("file_sharing"));
        assert!(!v.is_on("remote_login") && !v.is_on("screen_sharing"));
        assert!(!v.is_on("nope"));
    }

    #[test]
    fn the_switches_ask_for_the_opposite_of_their_state() {
        let v = View::from_text("", "", |unit| unit == "sshd.service");
        let g = groups(&v);
        assert_eq!(g[0].items[0].action, Action::Sharing(Edit::ComputerName));
        assert_eq!(g[1].items.len(), 4);
        assert_eq!(g[1].items[0].action, Action::Sharing(Edit::Service { id: "remote_login", on: false }));
        assert_eq!(g[1].items[1].action, Action::Sharing(Edit::Service { id: "file_sharing", on: true }));
    }

    #[test]
    fn a_toggle_is_shown_before_the_next_load() {
        let mut v = View::from_text("", "", off);
        v.set_service("screen_sharing", true);
        assert!(v.is_on("screen_sharing"));
        assert!(!v.is_on("remote_login"));
        v.set_service("screen_sharing", false);
        assert!(!v.is_on("screen_sharing"));
    }

    #[test]
    fn screen_sharing_cannot_be_switched_on_before_an_account_is_chosen() {
        let v = View::from_text("", "", off);
        assert_eq!(v.screen_user, None);
        let p = apply(&Edit::Service { id: "screen_sharing", on: true }, &v).expect("no privd call").expect("the account prompt");
        assert_eq!(p.purpose, Purpose::ScreenSharingAccount { start: true });
        assert_eq!(p.ok, "Turn On");
        assert_eq!(p.initial, "");
    }

    #[test]
    fn the_account_row_shows_the_account_and_edits_it() {
        let mut v = View::from_text("", "", off);
        assert_eq!(groups(&v)[2].items[0].value, "Not Set");
        v.screen_user = Some("olof".into());
        let g = groups(&v);
        assert_eq!(g[2].items[0].label, "Screen Sharing Account");
        assert_eq!(g[2].items[0].value, "olof");
        assert_eq!(g[2].items[0].action, Action::Sharing(Edit::ScreenSharingAccount));
        let p = apply(&Edit::ScreenSharingAccount, &v).expect("no privd call").expect("a prompt");
        assert_eq!(p.purpose, Purpose::ScreenSharingAccount { start: false });
        assert_eq!(p.initial, "olof");
        assert!(super::super::forms::acceptable(&p.purpose, "olof") && !super::super::forms::acceptable(&p.purpose, "Olof Dev"));
    }

    #[test]
    fn the_screen_sharing_switch_is_the_packaged_unit() {
        let v = View::from_text("", "", |unit| unit == "bifrost-wayvnc.service");
        assert!(v.is_on("screen_sharing"));
        assert!(!v.is_on("remote_login"));
    }

    #[test]
    fn the_name_prompt_starts_on_the_current_name() {
        let v = View::from_text("PRETTY_HOSTNAME=Hub\n", "", off);
        let p = apply(&Edit::ComputerName, &v).expect("no privd call").expect("a prompt");
        assert_eq!(p.purpose, Purpose::ComputerName);
        assert_eq!(p.initial, "Hub");
    }
}
