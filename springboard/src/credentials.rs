//! The credential store: the one shell-side API over the shared passwords group (`is.bifrost.passwords`), for every
//! caller that reads or removes what apps saved there. The Settings Passwords page, the `keychain-groups` service and
//! the SDK all name the group, the item kinds and the item names through this module, so there is one definition.
//!
//! Every item is a keychain item `(service, account)` in the group. The kind is carried by the service:
//! - a login: the service is the site (never starts with a kind prefix), the account is the user name, the value
//!   is the password;
//! - a secure note: the service is `note:` and the title, the account is [`NOTE_ACCOUNT`], the value is the body;
//! - a certificate: the service is `cert:` and the label, the account is [`CERTIFICATE_ACCOUNT`], the value is the
//!   PEM text.
//!
//! The prefixes contain a colon, which a site name cannot hold, so a login is never mistaken for a note or a
//! certificate. Every read goes through the group's own encrypted file (`keychain::Store::open_group`); an unreadable
//! group lists as empty and a missing secret is `None`. The shell acts for the user here, so it needs no app
//! membership to remove an item.

use crate::keychain::{Error, Store};
use std::path::Path;

/// The keychain access group that holds every saved login, note and certificate.
pub const GROUP: &str = "is.bifrost.passwords";
/// The service prefix of a secure note; the rest of the service is the note's title.
pub const NOTE_PREFIX: &str = "note:";
/// The service prefix of a certificate; the rest of the service is the certificate's label.
pub const CERTIFICATE_PREFIX: &str = "cert:";
/// The account every secure note is stored under.
pub const NOTE_ACCOUNT: &str = "secure-note";
/// The account every certificate is stored under.
pub const CERTIFICATE_ACCOUNT: &str = "certificate";

/// What an item in the passwords group is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Login,
    SecureNote,
    Certificate,
}

/// The kind of the item stored under `service`.
pub fn kind_of(service: &str) -> Kind {
    if service.starts_with(NOTE_PREFIX) {
        Kind::SecureNote
    } else if service.starts_with(CERTIFICATE_PREFIX) {
        Kind::Certificate
    } else {
        Kind::Login
    }
}

/// The service a secure note titled `title` is stored under.
pub fn note_service(title: &str) -> String {
    format!("{NOTE_PREFIX}{title}")
}

/// The service a certificate labelled `label` is stored under.
pub fn certificate_service(label: &str) -> String {
    format!("{CERTIFICATE_PREFIX}{label}")
}

/// The name an item shows as: the site of a login, the title of a note, the label of a certificate.
pub fn name_of(service: &str) -> &str {
    match kind_of(service) {
        Kind::Login => service,
        Kind::SecureNote => &service[NOTE_PREFIX.len()..],
        Kind::Certificate => &service[CERTIFICATE_PREFIX.len()..],
    }
}

/// One item of the group as a listing shows it: no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    /// The keychain service, which carries the kind (see the module comment).
    pub service: String,
    pub account: String,
    /// Unix time the item was last written (0 when unknown).
    pub modified: u64,
}

impl Item {
    /// The name shown in a list: [`name_of`] the service.
    pub fn name(&self) -> &str {
        name_of(&self.service)
    }
}

/// A saved website login as the Settings Passwords page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub site: String,
    pub account: String,
    /// Unix time the login was last written (0 when unknown).
    pub modified: u64,
}

fn group(root: &Path) -> Result<Store, Error> {
    Store::open_group(root, GROUP)
}

/// Every item in the passwords group, sorted by service then account. An unreadable group lists as empty.
pub fn items(root: &Path) -> Vec<Item> {
    let rows = group(root).and_then(|s| s.list()).unwrap_or_default();
    rows.into_iter().map(|r| Item { kind: kind_of(&r.service), service: r.service, account: r.account, modified: r.modified }).collect()
}

/// Every saved login, sorted by site then account.
pub fn logins(root: &Path) -> Vec<Login> {
    items(root)
        .into_iter()
        .filter(|i| i.kind == Kind::Login)
        .map(|i| Login { site: i.service, account: i.account, modified: i.modified })
        .collect()
}

/// The secret of one item (a password, a note body or a certificate's PEM text), or `None` when it is missing or not
/// text.
pub fn secret(root: &Path, service: &str, account: &str) -> Option<String> {
    let value = group(root).and_then(|s| s.get(service, account)).ok()?;
    String::from_utf8(value).ok()
}

/// Deletes one item of the passwords group, of any kind.
pub fn remove(root: &Path, service: &str, account: &str) -> Result<(), Error> {
    group(root)?.delete(service, account)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-credentials-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_kind_comes_from_the_service_prefix() {
        assert_eq!(kind_of("a.example"), Kind::Login);
        assert_eq!(kind_of("localhost"), Kind::Login);
        assert_eq!(kind_of(&note_service("Wi-Fi codes")), Kind::SecureNote);
        assert_eq!(kind_of(&certificate_service("Work CA")), Kind::Certificate);
        assert_eq!(kind_of("note"), Kind::Login, "a site named like the prefix without the colon is a login");
        assert_eq!(Kind::default(), Kind::Login);
    }

    #[test]
    fn the_name_is_the_title_or_label_without_its_prefix() {
        assert_eq!(name_of("a.example"), "a.example");
        assert_eq!(name_of(&note_service("Wi-Fi codes")), "Wi-Fi codes");
        assert_eq!(name_of(&certificate_service("Work CA")), "Work CA");
        assert_eq!(name_of(&note_service("")), "", "an empty title is a note with no name");
    }

    #[test]
    fn the_group_holds_logins_notes_and_certificates_and_removes_any_of_them() {
        let d = dir("kinds");
        let store = group(&d).unwrap();
        store.add("b.example", "bo", b"hunter3").unwrap();
        store.add("a.example", "alice", b"hunter2").unwrap();
        store.add(&note_service("Wi-Fi codes"), NOTE_ACCOUNT, b"1234").unwrap();
        store.add(&certificate_service("Work CA"), CERTIFICATE_ACCOUNT, b"-----BEGIN CERTIFICATE-----").unwrap();

        let all = items(&d);
        assert_eq!(all.len(), 4);
        assert_eq!(all.iter().filter(|i| i.kind == Kind::Login).count(), 2);
        assert!(all.iter().any(|i| i.kind == Kind::SecureNote && i.name() == "Wi-Fi codes"));
        assert!(all.iter().any(|i| i.kind == Kind::Certificate && i.name() == "Work CA"));

        let rows = logins(&d);
        let sites: Vec<(&str, &str)> = rows.iter().map(|l| (l.site.as_str(), l.account.as_str())).collect();
        assert_eq!(sites, vec![("a.example", "alice"), ("b.example", "bo")], "the login page lists logins only");
        assert_eq!(secret(&d, "a.example", "alice").as_deref(), Some("hunter2"));
        assert_eq!(secret(&d, &note_service("Wi-Fi codes"), NOTE_ACCOUNT).as_deref(), Some("1234"));
        assert_eq!(secret(&d, "a.example", "nobody"), None);

        remove(&d, "a.example", "alice").unwrap();
        remove(&d, &note_service("Wi-Fi codes"), NOTE_ACCOUNT).unwrap();
        assert_eq!(items(&d).len(), 2);
        assert_eq!(remove(&d, "a.example", "alice"), Err(Error::ItemNotFound));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_empty_or_unknown_store_lists_nothing() {
        let d = dir("empty");
        assert!(items(&d).is_empty());
        assert!(logins(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
