//! Mail's accounts are the mail accounts of the shared Internet Accounts list that Settings > Internet Accounts keeps
//! (`bifrost_app::controls::internet_accounts`). Mail reads and writes that list and nothing else. Only the accounts
//! with the Mail service switched on are used. The passwords are not in the list: they live in the passwords group
//! under each account's `password_site()` and `username` (see `bifrost_app::passwords`).

use bifrost_app::controls::internet_accounts::{self as shared, Account, Server, Service};
use std::path::PathBuf;

/// The whole shared list, every kind of account. A missing file is an empty list; a damaged one is an error the window
/// shows.
pub fn load_all() -> Result<Vec<Account>, String> {
    let Some(path) = shared::store_path() else { return Ok(Vec::new()) };
    shared::load(&path).map_err(|e| format!("{}: {e}", shared::STORE_FILE))
}

/// The mail accounts the user switched on, in list order.
pub fn load_mail() -> Result<Vec<Account>, String> {
    let all = load_all()?;
    Ok(shared::serving(&all, Service::Mail).into_iter().cloned().collect())
}

/// Adds an account to the shared list and writes the list. Refuses an account that does not validate, and an id the
/// list already holds.
pub fn add(account: Account) -> Result<(), String> {
    let path: PathBuf = shared::store_path().ok_or("There is no data directory for accounts")?;
    let mut all = load_all()?;
    if all.iter().any(|a| a.id == account.id) {
        return Err(format!("{} is already in the account list", account.name));
    }
    all.push(account);
    shared::save(&path, &all).map_err(|e| format!("{}: {e}", shared::STORE_FILE))
}

/// The incoming (IMAP) server of a mail account. Accounts from the list always have one.
pub fn incoming(account: &Account) -> Result<&Server, String> {
    account.imap.as_ref().ok_or_else(|| format!("{} has no incoming mail server", account.name))
}

/// The outgoing (SMTP) server of a mail account. Accounts from the list always have one.
pub fn outgoing(account: &Account) -> Result<&Server, String> {
    account.smtp.as_ref().ok_or_else(|| format!("{} has no outgoing mail server", account.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::internet_accounts::{Kind, Security};

    #[test]
    fn a_mail_account_without_servers_is_an_error_not_a_panic() {
        let a = Account::new(Kind::Mail, "work", "Work", "olibuijr");
        assert!(incoming(&a).is_err());
        assert!(outgoing(&a).is_err());
        let mut b = a.clone();
        b.imap = Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls });
        assert_eq!(incoming(&b).unwrap().port, 993);
    }
}
