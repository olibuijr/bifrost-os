//! Settings > Internet Accounts: the mail, calendar and contact accounts that Mail, Calendar, Contacts and Reminders
//! read from the shared list (`bifrost_ui::internet_accounts`).
//!
//! Each account lists its services with a switch per service (Mail; Calendars and Reminders for CalDAV; Contacts for
//! CardDAV), then a Remove row. Add Account asks for the address, the user name and the password in three one-field
//! prompts ([`super::forms`]). The password goes to the passwords group; the list file never holds one.
//!
//! The page is built from the list read at build time, so it needs no state of its own. The file operations here
//! take the list path and the keychain root as arguments, and the pure parts are tested without a disk.

use super::forms::{Prompt, Purpose};
use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use bifrost_ui::account_store::{self as store, Contents};
use bifrost_ui::internet_accounts::{self as shared, Account, Kind, Security, Server, Service};
use std::path::{Path, PathBuf};

/// A tap on an Internet Accounts row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Start adding an account of this kind (opens the first prompt).
    Add(Kind),
    /// Switch one service of an account on or off.
    Toggle { id: String, service: Service },
    /// Remove an account and its saved password.
    Remove { id: String },
    /// Clear a refused account-type request, so the app may ask again.
    ForgetRefusal { app: String, account_type: String },
}

/// What the page is built from.
pub struct PageState<'a> {
    pub accounts: &'a [Account],
    /// The accounts saved by apps in the accounts store, listed read only under their account types.
    pub store: &'a Contents,
    /// Why the last change did not happen (empty when nothing failed).
    pub note: &'a str,
}

/// The Internet Accounts page.
pub fn spec(s: &PageState) -> Spec {
    let mut groups = Vec::new();
    if !s.note.is_empty() {
        groups.push(Group::new(vec![Item::new(s.note).tint(Tint::Red)]));
    }
    if s.accounts.is_empty() {
        groups.push(Group::new(vec![Item::new("No Accounts").tint(Tint::Dim)]).footer("Add an account below. Mail, Calendar, Contacts and Reminders use the accounts that are switched on."));
    }
    for a in s.accounts {
        let rows: Vec<Item> = a
            .kind
            .services()
            .iter()
            .map(|&service| Item::new(service.label()).check(a.has(service), Action::InternetAccounts(Act::Toggle { id: a.id.clone(), service })))
            .collect();
        groups.push(Group::new(rows).header(&a.name).footer(&detail(a)));
        groups.push(Group::new(vec![Item::new("Remove Account").tint(Tint::Red).action(Action::InternetAccounts(Act::Remove { id: a.id.clone() }))]));
    }
    // the accounts apps saved, under each account type the store holds (read only: an app manages its own)
    for t in &s.store.types {
        let saved = s.store.accounts_with_type(&t.identifier);
        if saved.is_empty() {
            continue;
        }
        let rows: Vec<Item> = saved
            .iter()
            .map(|a| Item::new(if a.account_description.is_empty() { a.username.clone() } else { a.account_description.clone() }))
            .collect();
        let access = if t.access_granted { "Apps may use these accounts." } else { "Apps may not use these accounts yet." };
        groups.push(Group::new(rows).header(&t.description).footer(&format!("Saved by apps. {access}")));
    }
    // the requests the person refused: an app asks again only after its row is cleared here
    if !s.store.refusals.is_empty() {
        let rows: Vec<Item> = s
            .store
            .refusals
            .iter()
            .map(|r| {
                let label = format!("{} · {}", r.app, s.store.type_description(&r.account_type));
                Item::new(label).action(Action::InternetAccounts(Act::ForgetRefusal { app: r.app.clone(), account_type: r.account_type.clone() }))
            })
            .collect();
        groups.push(Group::new(rows).header("DECLINED REQUESTS").footer("Apps that asked for an account type you refused. Tap one to let it ask again."));
    }
    let adds: Vec<Item> = Kind::ALL.iter().map(|&k| Item::new(k.label()).tint(Tint::Blue).action(Action::InternetAccounts(Act::Add(k)))).collect();
    groups.push(Group::new(adds).header("ADD ACCOUNT").footer("Mail reads IMAP and SMTP at the servers of the address's domain. Passwords are saved in Passwords."));
    Spec { id: PageId::InternetAccounts, title: "Internet Accounts".into(), large: false, groups }
}

/// The footer under an account: its kind, login name and address or server.
fn detail(a: &Account) -> String {
    let place = match a.kind {
        Kind::Mail => a.email.as_str(),
        Kind::CalDav | Kind::CardDav => a.url.as_str(),
    };
    format!("{}  ·  {}  ·  {}", a.kind.label(), a.username, place)
}

/// The accounts in the shared list and in the accounts store. An unreadable file shows as none; changes to the shared
/// list use [`load_for_change`] instead.
pub fn load() -> (Vec<Account>, Contents) {
    let accounts = shared::store_path().and_then(|p| shared::load(&p).ok()).unwrap_or_default();
    let saved = store::store_path().and_then(|p| store::read(&p).ok()).unwrap_or_default();
    (accounts, saved)
}

/// Where the shared list is kept, or why it has no place.
pub fn list_path() -> Result<PathBuf, String> {
    shared::store_path().ok_or_else(|| "The accounts file has no home folder".to_string())
}

/// The list to change. A list that does not read is refused, so a change never overwrites a file it could not read.
fn load_for_change(path: &Path) -> Result<Vec<Account>, String> {
    shared::load(path).map_err(|e| format!("The accounts file could not be read: {e}"))
}

/// Switches one service of an account on or off.
pub fn toggle(path: &Path, id: &str, service: Service) -> Result<(), String> {
    let mut all = load_for_change(path)?;
    let account = all.iter_mut().find(|a| a.id == id).ok_or_else(|| "That account is gone.".to_string())?;
    let on = !account.has(service);
    account.set_service(service, on);
    shared::save(path, &all).map_err(|e| format!("The change could not be saved: {e}"))
}

/// Clears a refused request from the accounts store, so the app is asked again the next time it asks.
pub fn forget_refusal(app: &str, account_type: &str) -> Result<(), String> {
    let path = store::store_path().ok_or_else(|| "The accounts file has no home folder".to_string())?;
    store::remove_refusal(&path, app, account_type).map(|_| ()).map_err(|e| format!("The change could not be saved: {e}"))
}

/// Removes an account from the list and its password from the passwords group.
pub fn remove(path: &Path, root: &Path, id: &str) -> Result<(), String> {
    let mut all = load_for_change(path)?;
    let pos = all.iter().position(|a| a.id == id).ok_or_else(|| "That account is gone.".to_string())?;
    let gone = all.remove(pos);
    shared::save(path, &all).map_err(|e| format!("The change could not be saved: {e}"))?;
    let _ = crate::credentials::remove(root, &gone.password_site(), &gone.username);
    Ok(())
}

/// Whether `purpose` is one of the three Add Account prompts.
pub fn owns(purpose: &Purpose) -> bool {
    matches!(purpose, Purpose::NewAccountAddress { .. } | Purpose::NewAccountUser { .. } | Purpose::NewAccountPassword { .. })
}

/// One submitted Add Account prompt. Answers the next prompt of the flow, `None` once the account is added, or the
/// reason it was refused (the prompt is then shown again with the reason on the page).
pub fn finish(path: &Path, root: &Path, purpose: &Purpose, text: &str) -> Result<Option<Prompt>, String> {
    match purpose {
        Purpose::NewAccountAddress { kind } => {
            let address = text.trim();
            let ok = match kind {
                Kind::Mail => shared::valid_address(address),
                Kind::CalDav | Kind::CardDav => shared::valid_dav_url(address),
            };
            if !ok {
                return Err(match kind {
                    Kind::Mail => "Enter an email address, such as name@example.com.".to_string(),
                    Kind::CalDav | Kind::CardDav => "Enter a web address that starts with https://.".to_string(),
                });
            }
            Ok(Some(Prompt::new_account_user(*kind, address)))
        }
        Purpose::NewAccountUser { kind, address } => Ok(Some(Prompt::new_account_password(*kind, address, text.trim()))),
        Purpose::NewAccountPassword { kind, address, username } => {
            add(path, root, *kind, address, username, text)?;
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Adds the account described by the three prompts, then saves its password.
fn add(path: &Path, root: &Path, kind: Kind, address: &str, username: &str, password: &str) -> Result<(), String> {
    let mut all = load_for_change(path)?;
    let id = fresh_id(&all, kind);
    let account = build(kind, &id, address, username)?;
    all.push(account.clone());
    shared::save(path, &all).map_err(|e| format!("The account could not be saved: {e}"))?;
    crate::credentials::save(root, &account.password_site(), &account.username, password).map_err(|_| "The password could not be saved in Passwords.".to_string())
}

/// The account for an address (mail) or server URL (CalDAV, CardDAV). Mail takes its servers from the address's
/// domain: IMAP on 993 with TLS and SMTP on 587 with STARTTLS. The DAV account is named by its host.
pub fn build(kind: Kind, id: &str, address: &str, username: &str) -> Result<Account, String> {
    let account = match kind {
        Kind::Mail => {
            let domain = address.rsplit('@').next().unwrap_or("");
            Account {
                email: address.to_string(),
                imap: Some(Server { host: format!("imap.{domain}"), port: 993, security: Security::Tls }),
                smtp: Some(Server { host: format!("smtp.{domain}"), port: 587, security: Security::StartTls }),
                ..Account::new(kind, id, address, username)
            }
        }
        Kind::CalDav | Kind::CardDav => {
            let host = shared::url_host(address).unwrap_or("").to_string();
            Account { url: address.to_string(), ..Account::new(kind, id, &host, username) }
        }
    };
    account.validate().map_err(|e| format!("The account is not valid: {e}"))?;
    Ok(account)
}

/// The first id of the form `kind-N` that no account in `all` uses.
pub fn fresh_id(all: &[Account], kind: Kind) -> String {
    (1..).map(|n| format!("{}-{n}", kind.as_str())).find(|id| !all.iter().any(|a| a.id == *id)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail() -> Account {
        build(Kind::Mail, "mail-1", "olibuijr@example.com", "olibuijr").unwrap()
    }

    fn caldav() -> Account {
        build(Kind::CalDav, "caldav-1", "https://dav.example.com/dav/", "olibuijr").unwrap()
    }

    #[test]
    fn accounts_apps_saved_are_listed_under_their_type() {
        use bifrost_ui::account_store::{AccountEntry, TypeEntry};
        let saved = Contents {
            refusals: Vec::new(),
            next_id: 2,
            types: vec![TypeEntry { identifier: "bifrost.mail".into(), description: "Mail".into(), access_granted: true }],
            accounts: vec![AccountEntry {
                identifier: "account-1".into(),
                account_type: "bifrost.mail".into(),
                username: "ann".into(),
                user_full_name: String::new(),
                account_description: String::new(),
            }],
        };
        let empty = spec(&PageState { accounts: &[], store: &Contents::default(), note: "" });
        let listed = spec(&PageState { accounts: &[], store: &saved, note: "" });
        assert_eq!(listed.groups.len(), empty.groups.len() + 1, "one group for the one type with accounts");
        let other_type = Contents { types: saved.types.clone(), ..Contents::default() };
        assert_eq!(spec(&PageState { accounts: &[], store: &other_type, note: "" }).groups.len(), empty.groups.len(), "a type without accounts adds no group");
    }

    #[test]
    fn a_refused_request_is_listed_and_clearing_it_is_an_action() {
        use bifrost_ui::account_store::RefusalEntry;
        let refused = Contents {
            refusals: vec![RefusalEntry { app: "app.notes".into(), account_type: "bifrost.mail".into() }],
            ..Contents::default()
        };
        let empty = spec(&PageState { accounts: &[], store: &Contents::default(), note: "" });
        let s = spec(&PageState { accounts: &[], store: &refused, note: "" });
        assert_eq!(s.groups.len(), empty.groups.len() + 1, "one group for the declined requests");
        let row = &s.groups[1].items[0];
        assert_eq!(row.action, Action::InternetAccounts(Act::ForgetRefusal { app: "app.notes".into(), account_type: "bifrost.mail".into() }));
    }

    #[test]
    fn an_empty_list_says_so_and_offers_three_kinds() {
        let s = spec(&PageState { accounts: &[], store: &Contents::default(), note: "" });
        assert_eq!(s.id, PageId::InternetAccounts);
        assert_eq!(s.title, "Internet Accounts");
        assert_eq!(s.groups[0].items[0].label, "No Accounts");
        let adds = s.groups.last().unwrap();
        assert_eq!(adds.header.as_deref(), Some("ADD ACCOUNT"));
        let kinds: Vec<Action> = adds.items.iter().map(|i| i.action.clone()).collect();
        assert_eq!(kinds, vec![Action::InternetAccounts(Act::Add(Kind::Mail)), Action::InternetAccounts(Act::Add(Kind::CalDav)), Action::InternetAccounts(Act::Add(Kind::CardDav))]);
    }

    #[test]
    fn a_caldav_account_has_calendar_and_reminder_switches() {
        let accounts = [caldav()];
        let s = spec(&PageState { accounts: &accounts, store: &Contents::default(), note: "" });
        let services = &s.groups[0];
        assert_eq!(services.header.as_deref(), Some("dav.example.com"));
        let labels: Vec<&str> = services.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["Calendars", "Reminders"]);
        assert!(services.items.iter().all(|i| i.accessory == super::super::list::Accessory::Check(true)), "both services start on");
        assert_eq!(services.items[1].action, Action::InternetAccounts(Act::Toggle { id: "caldav-1".into(), service: Service::Reminders }));
        assert_eq!(s.groups[1].items[0].action, Action::InternetAccounts(Act::Remove { id: "caldav-1".into() }));
    }

    #[test]
    fn a_note_shows_first() {
        let s = spec(&PageState { accounts: &[], store: &Contents::default(), note: "The change could not be saved." });
        assert_eq!(s.groups[0].items[0].label, "The change could not be saved.");
    }

    #[test]
    fn mail_takes_its_servers_from_the_domain() {
        let a = mail();
        assert_eq!(a.imap.as_ref().unwrap().host, "imap.example.com");
        assert_eq!(a.imap.as_ref().unwrap().port, 993);
        assert_eq!(a.smtp.as_ref().unwrap().security, Security::StartTls);
        assert_eq!(a.name, "olibuijr@example.com");
        assert_eq!(a.password_site(), "imap.example.com");
    }

    #[test]
    fn a_bad_address_is_refused_before_the_account_exists() {
        assert!(build(Kind::Mail, "mail-1", "not-an-address", "x").is_err());
        assert!(build(Kind::CardDav, "carddav-1", "ftp://example.com/", "x").is_err());
    }

    #[test]
    fn the_flow_asks_address_then_user_then_password() {
        let path = Path::new("/nonexistent/internet-accounts-test.ini");
        let root = Path::new("/nonexistent");
        let first = finish(path, root, &Purpose::NewAccountAddress { kind: Kind::Mail }, "bo@example.com").unwrap().unwrap();
        assert_eq!(first.purpose, Purpose::NewAccountUser { kind: Kind::Mail, address: "bo@example.com".into() });
        assert_eq!(first.initial, "bo@example.com", "a mail login starts as the address");
        let second = finish(path, root, &first.purpose, "bo").unwrap().unwrap();
        assert!(second.secure);
        assert_eq!(second.purpose, Purpose::NewAccountPassword { kind: Kind::Mail, address: "bo@example.com".into(), username: "bo".into() });
    }

    #[test]
    fn a_refused_address_says_why() {
        let path = Path::new("/nonexistent/internet-accounts-test.ini");
        let err = finish(path, path, &Purpose::NewAccountAddress { kind: Kind::CalDav }, "dav.example.com").unwrap_err();
        assert!(err.contains("https://"), "{err}");
    }

    #[test]
    fn a_toggle_and_a_remove_change_the_file() {
        let dir = std::env::temp_dir().join(format!("ia-page-{}", std::process::id()));
        let path = dir.join(shared::STORE_FILE);
        shared::save(&path, &[caldav()]).unwrap();
        toggle(&path, "caldav-1", Service::Reminders).unwrap();
        assert!(!shared::load(&path).unwrap()[0].has(Service::Reminders));
        assert!(toggle(&path, "gone", Service::Calendars).is_err());
        remove(&path, &dir, "caldav-1").unwrap();
        assert!(shared::load(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_id_skips_the_ones_in_use() {
        assert_eq!(fresh_id(&[], Kind::Mail), "mail-1");
        assert_eq!(fresh_id(&[mail()], Kind::Mail), "mail-2");
        assert_eq!(fresh_id(&[caldav()], Kind::Mail), "mail-1");
    }
}
