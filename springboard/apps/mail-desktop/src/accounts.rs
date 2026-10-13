//! Mail's accounts come from the shared Internet Accounts list that Settings > Internet Accounts keeps. Only the
//! accounts with the Mail service switched on are used. The passwords are not in the list: they are read from the
//! passwords group under each account's [`password_site`](bifrost_app::controls::internet_accounts::Account::password_site)
//! and [`username`](bifrost_app::controls::internet_accounts::Account::username).

use bifrost_app::controls::internet_accounts::{self as shared, Account, Service};

/// The mail accounts the user switched on, in list order. An unreadable or missing list gives none.
pub fn mail_accounts() -> Vec<Account> {
    let Some(path) = shared::store_path() else { return Vec::new() };
    match shared::load(&path) {
        Ok(all) => shared::serving(&all, Service::Mail).into_iter().cloned().collect(),
        Err(_) => Vec::new(),
    }
}
