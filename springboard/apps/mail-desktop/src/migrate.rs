//! The first run of Mail 0.4.0 or later moves the accounts Mail 0.3.0 kept in its own `accounts.ini` (the app data
//! directory) into the shared Internet Accounts list, once. The passwords follow: 0.3.0 kept them in the keychain under
//! [`LEGACY_SERVICE`], and the window moves each one to the passwords group (see `window`). The old file is renamed
//! afterwards, so it is never read again, and an account already in the list (same id or same address) is not added twice.

use crate::accounts;
use bifrost_app::controls::internet_accounts::{self as shared, Account, Kind};
use std::io;
use std::path::Path;

/// The file 0.3.0 saved its accounts in, inside the app data directory.
pub const LEGACY_FILE: &str = "accounts.ini";
/// The name the file takes once its accounts are in the shared list.
pub const MIGRATED_FILE: &str = "accounts.ini.migrated";
/// The keychain service 0.3.0 stored the passwords under (the app's id). The account name is [`legacy_name`].
pub const LEGACY_SERVICE: &str = "is.olibuijr.mail.desktop";

/// The keychain account name 0.3.0 used for an account's password.
pub fn legacy_name(legacy_id: &str) -> String {
    format!("account:{legacy_id}")
}

/// An account read from the old file, and the id it had there (its password is under that id in the keychain).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub legacy_id: String,
    pub account: Account,
}

/// The old file's text as the shared list reads it. 0.3.0 wrote only mail accounts and no `kind` line, so each section
/// gets `kind = mail`.
pub fn shared_text(legacy: &str) -> String {
    let mut out = String::with_capacity(legacy.len() + 64);
    for line in legacy.lines() {
        out.push_str(line);
        out.push('\n');
        if line.trim_start().starts_with("[account ") {
            out.push_str("kind = mail\n");
        }
    }
    out
}

/// The accounts of the old file that `taken` (the shared list) does not hold. An account whose id is taken gets a new
/// one; an account with the address of a mail account already listed is left out.
pub fn plan(legacy: &str, taken: &[Account]) -> Result<Vec<Imported>, String> {
    let parsed = shared::decode(&shared_text(legacy)).map_err(|e| format!("{LEGACY_FILE}: {e}"))?;
    let mut list: Vec<Account> = taken.to_vec();
    let mut out = Vec::new();
    for old in parsed {
        let listed = list.iter().any(|a| a.kind == Kind::Mail && a.email.eq_ignore_ascii_case(&old.email));
        if listed {
            continue;
        }
        let mut account = old.clone();
        account.id = unique_id(&old.id, &list);
        list.push(account.clone());
        out.push(Imported { legacy_id: old.id, account });
    }
    Ok(out)
}

fn unique_id(base: &str, taken: &[Account]) -> String {
    let mut id = base.to_string();
    let mut n = 2;
    while taken.iter().any(|a| a.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Runs the import in the data directory `data_dir`: reads the old file, adds its new accounts to the shared list in one
/// write, and renames the old file. Without an old file there is nothing to do. The old file stays where it is when the
/// list cannot be written, so the next run tries again.
pub fn run(data_dir: &Path) -> Result<Vec<Imported>, String> {
    let path = data_dir.join(LEGACY_FILE);
    let legacy = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{LEGACY_FILE}: {e}")),
    };
    let taken = accounts::load_all()?;
    let imported = plan(&legacy, &taken)?;
    if !imported.is_empty() {
        accounts::import(imported.iter().map(|i| i.account.clone()).collect())?;
    }
    std::fs::rename(&path, data_dir.join(MIGRATED_FILE)).map_err(|e| format!("{LEGACY_FILE}: {e}"))?;
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::internet_accounts::{Security, Server, Service};

    const OLD: &str = "[account work]\n\
name = Work\n\
email = olibuijr@example.com\n\
username = olibuijr\n\
imap.host = imap.example.com\n\
imap.port = 993\n\
imap.security = tls\n\
smtp.host = smtp.example.com\n\
smtp.port = 587\n\
smtp.security = starttls\n\
\n\
[account home]\n\
name = Home\n\
email = ana@example.com\n\
username = ana\n\
imap.host = imap.example.com\n\
imap.port = 993\n\
imap.security = tls\n\
smtp.host = smtp.example.com\n\
smtp.port = 587\n\
smtp.security = starttls\n";

    fn listed_mail(id: &str, email: &str) -> Account {
        let mut a = Account::new(Kind::Mail, id, "Listed", email);
        a.email = email.into();
        a.imap = Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls });
        a.smtp = Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls });
        a
    }

    #[test]
    fn the_old_sections_become_mail_accounts() {
        let imported = plan(OLD, &[]).unwrap();
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].legacy_id, "work");
        assert_eq!(imported[0].account.kind, Kind::Mail);
        assert_eq!(imported[0].account.email, "olibuijr@example.com");
        assert_eq!(imported[0].account.password_site(), "imap.example.com");
        assert!(imported[0].account.has(Service::Mail));
    }

    #[test]
    fn an_account_already_in_the_list_is_not_added_again() {
        let taken = vec![listed_mail("settings-1", "OLIBUIJR@example.com")];
        let imported = plan(OLD, &taken).unwrap();
        assert_eq!(imported.iter().map(|i| i.legacy_id.as_str()).collect::<Vec<_>>(), vec!["home"]);
    }

    #[test]
    fn a_taken_id_gets_a_new_one() {
        let taken = vec![listed_mail("work", "other@example.com")];
        let imported = plan(OLD, &taken).unwrap();
        assert_eq!(imported[0].account.id, "work-2");
        assert_eq!(imported[0].legacy_id, "work", "the keychain item keeps the old name");
    }

    #[test]
    fn a_damaged_old_file_is_an_error_and_imports_nothing() {
        assert!(plan("[account x]\nname = y\n", &[]).is_err());
        assert_eq!(plan("", &[]).unwrap(), Vec::new());
    }

    #[test]
    fn the_legacy_keychain_name_matches_what_0_3_0_wrote() {
        assert_eq!(legacy_name("work"), "account:work");
        assert_eq!(LEGACY_SERVICE, "is.olibuijr.mail.desktop");
    }

    #[test]
    fn the_file_names_are_fixed() {
        assert_eq!(LEGACY_FILE, "accounts.ini");
        assert_eq!(MIGRATED_FILE, "accounts.ini.migrated");
    }
}
