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

/// The list with the account of the same id replaced by `account`. An id the list lacks is an error.
fn replaced(mut all: Vec<Account>, account: Account) -> Result<Vec<Account>, String> {
    let at = all.iter().position(|a| a.id == account.id).ok_or_else(|| format!("{} is not in the account list", account.name))?;
    all[at] = account;
    Ok(all)
}

/// The list without the account with this id. An id the list lacks is an error.
fn removed(mut all: Vec<Account>, id: &str) -> Result<Vec<Account>, String> {
    let at = all.iter().position(|a| a.id == id).ok_or_else(|| format!("{id} is not in the account list"))?;
    all.remove(at);
    Ok(all)
}

/// Writes an edited account into the shared list in place of the one with its id (Account Settings).
pub fn update(account: Account) -> Result<(), String> {
    let path: PathBuf = shared::store_path().ok_or("There is no data directory for accounts")?;
    let all = replaced(load_all()?, account)?;
    shared::save(&path, &all).map_err(|e| format!("{}: {e}", shared::STORE_FILE))
}

/// Takes the account with this id out of the shared list (Remove Account).
pub fn remove(id: &str) -> Result<(), String> {
    let path: PathBuf = shared::store_path().ok_or("There is no data directory for accounts")?;
    let all = removed(load_all()?, id)?;
    shared::save(&path, &all).map_err(|e| format!("{}: {e}", shared::STORE_FILE))
}

/// Adds several accounts to the shared list in one write (the first-run import). Returns how many were added.
pub fn import(new: Vec<Account>) -> Result<usize, String> {
    let path: PathBuf = shared::store_path().ok_or("There is no data directory for accounts")?;
    let mut all = load_all()?;
    let added = new.len();
    all.extend(new);
    shared::save(&path, &all).map_err(|e| format!("{}: {e}", shared::STORE_FILE))?;
    Ok(added)
}

/// True when another account of `all` keeps its password in the same Passwords item as `account`: the same site and
/// the same user name. Such an item is shared, so removing or overwriting it for one account would break the other.
pub fn shares_password(all: &[Account], account: &Account) -> bool {
    let site = account.password_site();
    !site.is_empty() && all.iter().any(|a| a.id != account.id && a.password_site() == site && a.username == account.username)
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
    fn an_account_is_replaced_or_removed_by_its_id_and_a_missing_id_is_an_error() {
        let work = Account::new(Kind::Mail, "work", "Work", "me");
        let home = Account::new(Kind::Mail, "home", "Home", "me");
        let renamed = Account::new(Kind::Mail, "work", "Office", "me");
        let list = replaced(vec![work.clone(), home.clone()], renamed).unwrap();
        assert_eq!(list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Office", "Home"]);
        assert!(replaced(vec![home.clone()], work.clone()).unwrap_err().contains("not in the account list"));
        let list = removed(vec![work, home.clone()], "work").unwrap();
        assert_eq!(list, vec![home]);
        assert!(removed(list, "work").is_err());
    }

    #[test]
    fn accounts_with_the_same_site_and_user_share_one_password_item() {
        let server = Server { host: "imap.example.com".into(), port: 993, security: Security::Tls };
        let mut work = Account::new(Kind::Mail, "work", "Work", "me");
        work.imap = Some(server.clone());
        let mut alias = Account::new(Kind::Mail, "alias", "Alias", "me");
        alias.imap = Some(server.clone());
        let mut other_user = Account::new(Kind::Mail, "other", "Other", "you");
        other_user.imap = Some(server);
        let all = vec![work.clone(), alias.clone(), other_user.clone()];
        assert!(shares_password(&all, &work));
        assert!(shares_password(&all, &alias));
        assert!(!shares_password(&all, &other_user));
        assert!(!shares_password(&[work.clone()], &work), "an account does not share with itself");
        assert!(shares_password(&[work.clone()], &alias), "a new account is checked against the list");
    }

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
