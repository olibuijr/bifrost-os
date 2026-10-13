//! Matrix accounts live in the shared passwords group, next to the website logins, so Settings > Passwords lists them
//! and every app reads the same secret. An account is one login item: its service is `matrix:` followed by the
//! homeserver base address (`matrix:https://matrix.org`), its account is the Matrix user id, and its value is the
//! password. The access token is not stored; each start signs in again and makes a device.

use bifrost_app::keychain_groups::Row;

/// The service prefix of a Matrix login. A website login never starts with it, because a site name has no colon.
pub const SERVICE_PREFIX: &str = "matrix:";

/// A saved Matrix account: the homeserver base address and the user id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub homeserver: String,
    pub user: String,
}

/// The service a login for `homeserver` is stored under.
pub fn service(homeserver: &str) -> String {
    format!("{SERVICE_PREFIX}{homeserver}")
}

/// The account a passwords item is, when it is a Matrix login (`None` for every other item).
pub fn from_item(service_name: &str, account: &str) -> Option<Account> {
    let homeserver = service_name.strip_prefix(SERVICE_PREFIX)?;
    if !homeserver.starts_with("https://") || account.trim().is_empty() {
        return None;
    }
    Some(Account { homeserver: homeserver.to_string(), user: account.to_string() })
}

/// Every Matrix account in a passwords listing, in the listing's order.
pub fn from_rows(rows: &[Row]) -> Vec<Account> {
    rows.iter().filter_map(|row| from_item(&row.service, &row.account)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(service: &str, account: &str) -> Row {
        Row { service: service.into(), account: account.into(), modified: 0 }
    }

    #[test]
    fn a_login_round_trips_through_its_service_name() {
        let s = service("https://matrix.example.org");
        assert_eq!(s, "matrix:https://matrix.example.org");
        assert_eq!(
            from_item(&s, "@alice:example.org"),
            Some(Account { homeserver: "https://matrix.example.org".into(), user: "@alice:example.org".into() })
        );
    }

    #[test]
    fn website_logins_and_damaged_items_are_not_matrix_accounts() {
        assert_eq!(from_item("example.com", "alice"), None);
        assert_eq!(from_item("matrix:http://plain.example.org", "@a:b"), None, "only https homeservers");
        assert_eq!(from_item("matrix:https://matrix.org", "  "), None, "an item needs a user");
    }

    #[test]
    fn a_listing_keeps_only_the_matrix_accounts_in_order() {
        let rows = vec![
            row("example.com", "bob"),
            row("matrix:https://a.example", "@one:a.example"),
            row("note:Shopping", "secure-note"),
            row("matrix:https://b.example", "@two:b.example"),
        ];
        let accounts = from_rows(&rows);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].user, "@one:a.example");
        assert_eq!(accounts[1].homeserver, "https://b.example");
    }
}
