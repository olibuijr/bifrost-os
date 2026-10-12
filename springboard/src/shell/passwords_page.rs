//! Settings > Passwords: the website logins apps saved to the passwords group (`keychain_groups::PASSWORDS_GROUP`).
//!
//! The page lists each login by site, with the account and the time it was last written. Tapping a login reveals its
//! password: after the passcode when one is set (the Settings overlay does the asking), then the password shows in
//! the page with Copy, Hide and Delete rows. This module only builds the page from the rows the caller read, so it
//! needs no keychain and its tests run on the host.

use super::diag::date_text;
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use super::services::keychain_groups::Login;

/// What a tap on a Passwords row does. The shell carries it out in Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Show the password of this login (after the passcode, when one is set).
    Reveal { site: String, account: String },
    /// Copy the revealed password to the clipboard.
    Copy,
    /// Hide the revealed password again.
    Hide,
    /// Delete this login from the passwords group.
    Delete { site: String, account: String },
}

/// The login whose password is shown on the page, and the password itself.
pub struct Shown<'a> {
    pub site: &'a str,
    pub account: &'a str,
    pub password: &'a str,
}

/// What the page is built from.
pub struct PageState<'a> {
    /// Every login in the passwords group, already sorted by site then account.
    pub logins: &'a [Login],
    /// The login the user revealed, if any.
    pub revealed: Option<Shown<'a>>,
    /// Zone offset in seconds east of UTC, for the modified times.
    pub offset: i32,
    /// Whether revealing a password asks for the passcode first (only used for the footer text).
    pub passcode_on: bool,
}

/// The Passwords page.
pub fn spec(s: &PageState) -> Spec {
    let mut groups = Vec::new();
    if s.logins.is_empty() {
        groups.push(Group::new(vec![Item::new("No Saved Passwords").tint(Tint::Dim)]).footer("Logins that apps save with the passwords group appear here, sorted by site."));
    } else {
        let rows = s
            .logins
            .iter()
            .map(|l| {
                let value = format!("{}  ·  {}", l.account, date_text(l.modified as i64, s.offset));
                Item::new(l.site.as_str()).value(value).action(Action::Passwords(Act::Reveal { site: l.site.clone(), account: l.account.clone() }))
            })
            .collect();
        let footer = if s.passcode_on { "Tap a login to show its password. Your passcode is asked first." } else { "Tap a login to show its password." };
        groups.push(Group::new(rows).footer(footer));
    }
    if let Some(shown) = &s.revealed {
        groups.push(Group::new(vec![Item::new("Password").value(shown.password)]));
        groups.push(
            Group::new(vec![
                Item::new("Copy Password").tint(Tint::Blue).action(Action::Passwords(Act::Copy)),
                Item::new("Hide Password").tint(Tint::Blue).action(Action::Passwords(Act::Hide)),
                Item::new("Delete Password").tint(Tint::Red).action(Action::Passwords(Act::Delete { site: shown.site.to_string(), account: shown.account.to_string() })),
            ])
            .footer("Delete removes this login from every app that saves to the passwords group."),
        );
    }
    Spec { id: PageId::Passwords, title: "Passwords".into(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(site: &str, account: &str, modified: u64) -> Login {
        Login { site: site.into(), account: account.into(), modified }
    }

    #[test]
    fn an_empty_store_says_so_and_has_no_rows_to_tap() {
        let s = spec(&PageState { logins: &[], revealed: None, offset: 0, passcode_on: false });
        assert_eq!(s.id, PageId::Passwords);
        assert_eq!(s.title, "Passwords");
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].items[0].label, "No Saved Passwords");
        assert_eq!(s.groups[0].items[0].tint, Tint::Dim);
    }

    #[test]
    fn each_login_shows_site_then_account_and_modified_time() {
        // 1791073395 is Oct 4, 00:23 UTC (checked with date -u)
        let logins = [login("a.example", "alice", 1_791_073_395)];
        let s = spec(&PageState { logins: &logins, revealed: None, offset: 0, passcode_on: true });
        assert_eq!(s.groups.len(), 1);
        let row = &s.groups[0].items[0];
        assert_eq!(row.label, "a.example");
        assert_eq!(row.value, "alice  ·  Oct 4, 00:23");
        assert_eq!(row.action, Action::Passwords(Act::Reveal { site: "a.example".into(), account: "alice".into() }));
        assert!(s.groups[0].footer.as_deref().unwrap_or("").contains("passcode"));
    }

    #[test]
    fn a_revealed_login_adds_the_password_and_copy_hide_delete_rows() {
        let logins = [login("a.example", "alice", 0)];
        let shown = Shown { site: "a.example", account: "alice", password: "hunter2" };
        let s = spec(&PageState { logins: &logins, revealed: Some(shown), offset: 0, passcode_on: false });
        assert_eq!(s.groups.len(), 3);
        assert_eq!(s.groups[1].items[0].label, "Password");
        assert_eq!(s.groups[1].items[0].value, "hunter2");
        let labels: Vec<&str> = s.groups[2].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["Copy Password", "Hide Password", "Delete Password"]);
        assert_eq!(s.groups[2].items[2].action, Action::Passwords(Act::Delete { site: "a.example".into(), account: "alice".into() }));
        assert_eq!(s.groups[2].items[2].tint, Tint::Red);
    }
}
